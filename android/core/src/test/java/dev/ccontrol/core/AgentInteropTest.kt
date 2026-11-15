package dev.ccontrol.core

import dev.ccontrol.core.net.AgentSession
import dev.ccontrol.core.net.AppInfo
import dev.ccontrol.core.net.ConnectException
import dev.ccontrol.core.net.PairingFlow
import dev.ccontrol.core.net.PairingUpdate
import dev.ccontrol.core.net.RemoteException
import dev.ccontrol.core.protocol.Action
import dev.ccontrol.core.protocol.ActionResult
import dev.ccontrol.core.protocol.Message
import dev.ccontrol.core.protocol.Scopes
import dev.ccontrol.core.security.DeviceIdentity
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.bouncycastle.asn1.x500.X500Name
import org.bouncycastle.cert.jcajce.JcaX509CertificateConverter
import org.bouncycastle.cert.jcajce.JcaX509v3CertificateBuilder
import org.bouncycastle.operator.jcajce.JcaContentSignerBuilder
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import java.io.File
import java.math.BigInteger
import java.net.HttpURLConnection
import java.net.ServerSocket
import java.net.Socket
import java.net.URL
import java.security.KeyPairGenerator
import java.security.spec.ECGenParameterSpec
import java.util.Date

/**
 * Runs the Android controller stack (TLS, framing, commit/reveal pairing,
 * pinning, requests, revocation) on the JVM against the real Rust agent.
 *
 * Opt-in: `CCONTROL_AGENT_BIN=/path/to/ccontrol-agent ./gradlew :core:testDebugUnitTest`.
 * The phone identity here is a software P-256 key; on devices it lives in
 * Android Keystore (same algorithm, same certificate shape).
 */
class AgentInteropTest {
    private var agent: Process? = null
    private lateinit var dir: File
    private var port = 0
    private var adminPort = 0
    private lateinit var token: String
    private val app = AppInfo("Interop Phone", "test")

    private fun freePort() = ServerSocket(0).use { it.localPort }

    private fun softwareIdentity(): DeviceIdentity {
        val kp = KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.generateKeyPair()
        val name = X500Name("CN=ccontrol phone")
        val holder = JcaX509v3CertificateBuilder(name, BigInteger.ONE, Date(0), Date(4102444800000), name, kp.public)
            .build(JcaContentSignerBuilder("SHA256withECDSA").build(kp.private))
        return DeviceIdentity("test", kp.private, JcaX509CertificateConverter().getCertificate(holder))
    }

    @Before fun start() {
        val bin = System.getenv("CCONTROL_AGENT_BIN")
        assumeTrue("CCONTROL_AGENT_BIN not set; skipping interop test", bin != null && File(bin).canExecute())
        dir = kotlin.io.path.createTempDirectory("ccontrol-interop").toFile()
        port = freePort(); adminPort = freePort()
        File(dir, "config.json").writeText(
            """{"listen_addr":"127.0.0.1","listen_port":$port,"admin_port":$adminPort,"mdns_enabled":false}""",
        )
        agent = ProcessBuilder(bin, "--data-dir", dir.absolutePath, "run").redirectErrorStream(true)
            .redirectOutput(File(dir, "agent.out")).start()
        repeat(100) {
            if (runCatching { Socket("127.0.0.1", adminPort).close() }.isSuccess) {
                token = File(dir, "admin.token").readText().trim()
                return
            }
            Thread.sleep(100)
        }
        error("agent did not start: " + File(dir, "agent.out").readText())
    }

    @After fun stop() {
        agent?.destroy()
        agent?.waitFor()
    }

    private fun admin(method: String, path: String, body: String? = null): Pair<Int, String> {
        val c = URL("http://127.0.0.1:$adminPort$path").openConnection() as HttpURLConnection
        c.requestMethod = method
        c.setRequestProperty("x-ccontrol-token", token)
        c.setRequestProperty("content-type", "application/json")
        if (body != null) { c.doOutput = true; c.outputStream.use { it.write(body.toByteArray()) } }
        val code = c.responseCode
        val text = (if (code < 400) c.inputStream else c.errorStream)?.bufferedReader()?.readText() ?: ""
        return code to text
    }

    private suspend fun pendingSas(): Pair<Int, String> {
        repeat(100) {
            val (_, s) = admin("GET", "/api/state")
            Regex(""""request_id":(\d+).*?"sas":"(\d{6})"""").find(s)?.let { return it.groupValues[1].toInt() to it.groupValues[2] }
            delay(50)
        }
        error("no pending request")
    }

    @Test fun pairControlReconnectRevoke() = runBlocking {
        val me = softwareIdentity()

        // 1. First contact: unpaired, any server identity accepted.
        val s = AgentSession.open("127.0.0.1", port, me, null, app)
        assertFalse(s.helloAck.paired)
        val serverFp = s.serverFingerprint
        var phoneSas: String? = null
        val pairing = async(kotlinx.coroutines.Dispatchers.IO) {
            PairingFlow.run(s, me, Scopes.DEFAULT_REQUEST) { if (it is PairingUpdate.AwaitingApproval) phoneSas = it.sas }
        }
        val (reqId, desktopSas) = pendingSas()
        withTimeout(5000) { while (phoneSas == null) delay(20) }
        assertEquals("phone and desktop must show the same code", desktopSas, phoneSas)
        val (code, _) = admin("POST", "/api/pairing/$reqId/accept", """{"scopes":["device.read","telemetry.read"]}""")
        assertEquals(204, code)
        val result = pairing.await() as PairingUpdate.Accepted
        assertEquals(listOf("device.read", "telemetry.read"), result.grantedScopes)
        assertTrue(result.ack.paired)
        assertEquals(serverFp, result.ack.deviceId)

        // 2. Authorized requests on the same connection.
        s.startReader()
        val manifest = (s.request(Action.DeviceInfo) as ActionResult.Manifest).manifest
        assertEquals(serverFp, manifest.deviceId)
        assertTrue(manifest.capabilities.isNotEmpty())
        val tel = (s.request(Action.TelemetryGet) as ActionResult.TelemetryResult).telemetry
        assertTrue(tel.memoryTotalBytes > 0)
        // Phase 2: process list from the real OS (device.read), terminate denied.
        val procs = (s.request(Action.ProcessList) as ActionResult.Processes).processes
        assertTrue(procs.size > 5)
        val agentProc = procs.first { it.name.startsWith("ccontrol-agent") }
        assertFalse(agentProc.terminable)
        val noKill = runCatching { s.request(Action.ProcessTerminate(agentProc.pid, agentProc.name, true)) }.exceptionOrNull() as RemoteException
        assertEquals("scope_denied", noKill.error.code)
        assertTrue(tel.disks.isNotEmpty())
        // media.control was not granted -> typed denial, nothing executed.
        val denied = runCatching { s.request(Action.VolumeSet(0.5f)) }.exceptionOrNull() as RemoteException
        assertEquals("scope_denied", denied.error.code)
        s.close()

        // 3. Reconnect with the pinned fingerprint: no pairing again.
        val s2 = AgentSession.open("127.0.0.1", port, me, serverFp, app)
        assertTrue(s2.helloAck.paired)
        s2.startReader()

        // 4. Wrong pin is refused at the TLS layer.
        val wrong = runCatching { AgentSession.open("127.0.0.1", port, me, "0".repeat(64), app) }.exceptionOrNull()
        assertEquals(ConnectException.Kind.IdentityChanged, (wrong as ConnectException).kind)

        // 5. Revocation on the desktop disconnects the live session and removes trust.
        val myId = me.fingerprint
        val revoked = async(start = kotlinx.coroutines.CoroutineStart.UNDISPATCHED) { s2.events.first { it.event == "revoked" } }
        assertEquals(204, admin("POST", "/api/controllers/$myId/revoke").first)
        withTimeout(5000) { revoked.await() }
        withTimeout(5000) { s2.closed.await() }
        val s3 = AgentSession.open("127.0.0.1", port, me, serverFp, app)
        assertFalse("revoked phone must not be trusted", s3.helloAck.paired)
        s3.close()
    }

    @Test fun controllerCanUnpairItself() = runBlocking {
        val me = softwareIdentity()
        val s = AgentSession.open("127.0.0.1", port, me, null, app)
        val pairing = async(kotlinx.coroutines.Dispatchers.IO) { PairingFlow.run(s, me, emptyList()) {} }
        val (reqId, _) = pendingSas()
        admin("POST", "/api/pairing/$reqId/accept", """{"scopes":[]}""")
        assertTrue(pairing.await() is PairingUpdate.Accepted)
        s.startReader()
        val revoked = async(start = kotlinx.coroutines.CoroutineStart.UNDISPATCHED) { s.events.first { it.event == "revoked" } }
        assertEquals(ActionResult.Done, s.request(Action.Unpair))
        withTimeout(5000) { revoked.await() }
        val again = AgentSession.open("127.0.0.1", port, me, s.serverFingerprint, app)
        assertFalse(again.helloAck.paired)
        again.close()
    }

    @Test fun rejectedPairingLeavesNoTrust() = runBlocking {
        val me = softwareIdentity()
        val s = AgentSession.open("127.0.0.1", port, me, null, app)
        val pairing = async(kotlinx.coroutines.Dispatchers.IO) { PairingFlow.run(s, me, Scopes.DEFAULT_REQUEST) {} }
        val (reqId, _) = pendingSas()
        admin("POST", "/api/pairing/$reqId/reject")
        assertTrue(pairing.await() is PairingUpdate.Rejected)
        val again = AgentSession.open("127.0.0.1", port, me, s.serverFingerprint, app)
        assertFalse(again.helloAck.paired)
        again.close()
    }

    @Test fun adminApiRequiresToken() {
        val c = URL("http://127.0.0.1:$adminPort/api/state").openConnection() as HttpURLConnection
        assertEquals(403, c.responseCode)
        @Suppress("UNUSED_VARIABLE") val unused = Message.Ping(0)
    }
}
