package dev.ccontrol.core

import dev.ccontrol.core.net.AgentSession
import dev.ccontrol.core.net.AppInfo
import dev.ccontrol.core.net.PairingFlow
import dev.ccontrol.core.net.PairingUpdate
import dev.ccontrol.core.net.RemoteException
import dev.ccontrol.core.protocol.Action
import dev.ccontrol.core.protocol.ActionResult
import dev.ccontrol.core.protocol.RouteSource
import dev.ccontrol.core.security.DeviceIdentity
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.bouncycastle.asn1.x500.X500Name
import org.bouncycastle.cert.jcajce.JcaX509CertificateConverter
import org.bouncycastle.cert.jcajce.JcaX509v3CertificateBuilder
import org.bouncycastle.operator.jcajce.JcaContentSignerBuilder
import org.junit.After
import org.junit.Assert.assertEquals
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
 * Two real agent processes; the Kotlin controller stack prepares a sink on B,
 * starts a test-tone route on A, checks live stats, stops and verifies the
 * outcome. Opt-in via CCONTROL_AGENT_BIN, like AgentInteropTest.
 */
class AudioRouteInteropTest {
    private class Agent(val proc: Process, val dir: File, val port: Int, val admin: Int) {
        val token: String get() = File(dir, "admin.token").readText().trim()
    }

    private val agents = mutableListOf<Agent>()
    private val app = AppInfo("Interop Phone", "test")

    private fun freePort() = ServerSocket(0).use { it.localPort }

    private fun startAgent(bin: String): Agent {
        val dir = kotlin.io.path.createTempDirectory("ccontrol-audio").toFile()
        val port = freePort()
        val admin = freePort()
        File(dir, "config.json").writeText("""{"listen_addr":"127.0.0.1","listen_port":$port,"admin_port":$admin,"mdns_enabled":false}""")
        val p = ProcessBuilder(bin, "--data-dir", dir.absolutePath, "run").redirectErrorStream(true).redirectOutput(File(dir, "out.log")).start()
        repeat(100) {
            if (runCatching { Socket("127.0.0.1", admin).close() }.isSuccess) return Agent(p, dir, port, admin).also(agents::add)
            Thread.sleep(100)
        }
        error("agent did not start")
    }

    @Before fun check() {
        val bin = System.getenv("CCONTROL_AGENT_BIN")
        assumeTrue(bin != null && File(bin).canExecute())
    }

    @After fun stop() = agents.forEach { it.proc.destroy(); it.proc.waitFor() }

    private fun identity(): DeviceIdentity {
        val kp = KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.generateKeyPair()
        val name = X500Name("CN=ccontrol phone")
        val holder = JcaX509v3CertificateBuilder(name, BigInteger.ONE, Date(0), Date(4102444800000), name, kp.public)
            .build(JcaContentSignerBuilder("SHA256withECDSA").build(kp.private))
        return DeviceIdentity("test", kp.private, JcaX509CertificateConverter().getCertificate(holder))
    }

    private fun http(a: Agent, method: String, path: String, body: String? = null): String {
        val c = URL("http://127.0.0.1:${a.admin}$path").openConnection() as HttpURLConnection
        c.requestMethod = method
        c.setRequestProperty("x-ccontrol-token", a.token)
        c.setRequestProperty("content-type", "application/json")
        if (body != null) { c.doOutput = true; c.outputStream.use { it.write(body.toByteArray()) } }
        return (if (c.responseCode < 400) c.inputStream else c.errorStream)?.bufferedReader()?.readText() ?: ""
    }

    private suspend fun pair(a: Agent, me: DeviceIdentity): AgentSession = kotlinx.coroutines.coroutineScope {
        val s = AgentSession.open("127.0.0.1", a.port, me, null, app)
        val job = async(Dispatchers.IO) { PairingFlow.run(s, me, listOf("audio.route")) {} }
        var id: String? = null
        repeat(100) {
            id = Regex(""""request_id":(\d+)""").find(http(a, "GET", "/api/state"))?.groupValues?.get(1)
            if (id != null) return@repeat
            delay(50)
        }
        http(a, "POST", "/api/pairing/$id/accept", """{"scopes":["audio.route"]}""")
        assertTrue(job.await() is PairingUpdate.Accepted)
        s.startReader()
        s
    }

    @Test fun testToneRouteBetweenTwoRealAgents() = runBlocking {
        val bin = System.getenv("CCONTROL_AGENT_BIN")!!
        val a = startAgent(bin)
        val b = startAgent(bin)
        val me = identity()
        val src = pair(a, me)
        val dst = pair(b, me)

        // CI runners have no audio stack: skip rather than fail.
        val inv = runCatching { (src.request(Action.AudioDevices) as ActionResult.Audio).inventory }.getOrNull()
        assumeTrue("needs an output device", inv?.canPlay == true)

        val key = "ab".repeat(32)
        val sink = (dst.request(Action.SinkPrepare("kt-1", key, null, 15)) as ActionResult.Sink).sink
        assertEquals("waiting", sink.state)
        val route = (src.request(Action.RouteStart("kt-1", RouteSource.TestTone, "127.0.0.1", sink.udpPort, key, 15, false), 25_000) as ActionResult.Route).route
        assertEquals("route: $route", "active", route.state)
        assertEquals("test_tone", route.capture)

        delay(1500)
        val s2 = (dst.request(Action.RouteList) as ActionResult.Routes).sinks.first { it.routeId == "kt-1" }
        assertEquals("playing", s2.state)
        assertTrue("frames ${s2.stats}", s2.stats.framesPlayed > 50)
        assertEquals(0L, s2.stats.lost)
        val r2 = (src.request(Action.RouteList) as ActionResult.Routes).routes.first { it.routeId == "kt-1" }
        assertTrue("rtt reported ${r2.stats}", r2.stats.rttMs != null)

        assertEquals(ActionResult.Done, src.request(Action.LeaseRenew("kt-1")))
        val stopped = (src.request(Action.RouteStop("kt-1")) as ActionResult.Route).route
        assertEquals("idle", stopped.state)
        assertEquals("not_needed", stopped.outcome)
        assertEquals("stopped", (dst.request(Action.SinkStop("kt-1")) as ActionResult.Sink).sink.state)

        // Bad key length is rejected by validation on the agent.
        val bad = runCatching { dst.request(Action.SinkPrepare("kt-2", "abc", null, 15)) }.exceptionOrNull()
        assertTrue(bad is RemoteException)
        src.close(); dst.close()
    }
}
