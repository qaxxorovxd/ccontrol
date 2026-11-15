package dev.ccontrol.core.net

import dev.ccontrol.core.protocol.Action
import dev.ccontrol.core.protocol.ActionResult
import dev.ccontrol.core.protocol.Framing
import dev.ccontrol.core.protocol.Hello
import dev.ccontrol.core.protocol.HelloAck
import dev.ccontrol.core.protocol.Message
import dev.ccontrol.core.protocol.ProtocolError
import dev.ccontrol.core.protocol.ProtocolException
import dev.ccontrol.core.protocol.ProtocolInfo
import dev.ccontrol.core.security.DeviceIdentity
import dev.ccontrol.core.security.PinningTrustManager
import dev.ccontrol.core.security.TlsFactory
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.IOException
import java.net.InetSocketAddress
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong
import javax.net.ssl.SSLSocket

/** Agent returned a typed error for a request. */
class RemoteException(val error: ProtocolError) : Exception("${error.code}: ${error.message}")

/** Connection-level failure categories shown to the user. */
class ConnectException(val kind: Kind, message: String, cause: Throwable? = null) : Exception(message, cause) {
    enum class Kind { Unreachable, IdentityChanged, Incompatible, Rejected, Protocol }
}

data class AppInfo(val deviceName: String, val appVersion: String)

/**
 * One TLS connection to an agent. Created by [AgentSession.open]; the caller
 * either runs [PairingFlow] (unpaired) or calls [startReader] and then
 * [request] (paired).
 */
class AgentSession private constructor(
    private val socket: SSLSocket,
    private val input: DataInputStream,
    private val output: BufferedOutputStream,
    val serverFingerprint: String,
    val helloAck: HelloAck,
    val host: String,
    val port: Int,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val pending = ConcurrentHashMap<Long, CompletableDeferred<Message.Response>>()
    private val nextId = AtomicLong(0)
    private val _events = MutableSharedFlow<Message.Event>(replay = 4, extraBufferCapacity = 16, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    // replay: subscribers attached right after startReader() must not miss an early "revoked".
    val events: SharedFlow<Message.Event> = _events
    val closed = CompletableDeferred<Throwable?>()
    private var readerJob: Job? = null

    /** Blocking send of a single message (pairing steps). */
    internal fun sendBlocking(m: Message) = Framing.write(output, m)

    /** Blocking read of a single message (pairing steps), with socket timeout. */
    internal fun readBlocking(timeoutMs: Int): Message {
        socket.soTimeout = timeoutMs
        return Framing.read(input)
    }

    fun startReader() {
        if (readerJob != null) return
        socket.soTimeout = 0
        readerJob = scope.launch {
            var failure: Throwable? = null
            try {
                while (true) {
                    when (val m = Framing.read(input)) {
                        is Message.Response -> pending.remove(m.id)?.complete(m)
                        is Message.Event -> _events.emit(m)
                        is Message.Pong -> {}
                        is Message.Error -> throw RemoteException(m.error)
                        else -> throw ProtocolException("unexpected ${m::class.simpleName}")
                    }
                }
            } catch (t: Throwable) {
                failure = t
            } finally {
                pending.values.forEach { it.completeExceptionally(failure ?: IOException("closed")) }
                pending.clear()
                closed.complete(failure)
                runCatching { socket.close() }
            }
        }
    }

    suspend fun request(action: Action, timeoutMs: Long = 12_000): ActionResult {
        val id = nextId.incrementAndGet()
        val d = CompletableDeferred<Message.Response>()
        pending[id] = d
        try {
            withContext(Dispatchers.IO) { Framing.write(output, Message.Request(id, action)) }
            val r = withTimeout(timeoutMs) { d.await() }
            r.error?.let { throw RemoteException(it) }
            return r.result ?: throw ProtocolException("empty response")
        } finally {
            pending.remove(id)
        }
    }

    suspend fun ping(seq: Long) = withContext(Dispatchers.IO) { Framing.write(output, Message.Ping(seq)) }

    fun close() {
        runCatching { socket.close() }
        scope.cancel()
        closed.complete(null)
    }

    val isOpen: Boolean get() = !closed.isCompleted && !socket.isClosed

    companion object {
        /**
         * Connect, perform the TLS handshake and exchange hello.
         * [expectedFingerprint] pins the server (paired devices / QR); null
         * accepts any server identity and is only used during first pairing.
         */
        suspend fun open(
            host: String,
            port: Int,
            identity: DeviceIdentity,
            expectedFingerprint: String?,
            app: AppInfo,
            connectTimeoutMs: Int = 5_000,
        ): AgentSession = withContext(Dispatchers.IO) {
            val trust = PinningTrustManager(expectedFingerprint)
            val socket = TlsFactory.socketFactory(identity, trust).createSocket() as SSLSocket
            try {
                socket.enabledProtocols = arrayOf("TLSv1.3")
                socket.tcpNoDelay = true
                try {
                    socket.connect(InetSocketAddress(host, port), connectTimeoutMs)
                } catch (e: IOException) {
                    throw ConnectException(ConnectException.Kind.Unreachable, "Cannot reach $host:$port", e)
                }
                socket.soTimeout = 10_000
                try {
                    socket.startHandshake()
                } catch (e: IOException) {
                    if (expectedFingerprint != null && trust.observedFingerprint != null &&
                        !trust.observedFingerprint.equals(expectedFingerprint, true)
                    ) {
                        throw ConnectException(ConnectException.Kind.IdentityChanged, "This address now belongs to a different computer", e)
                    }
                    throw ConnectException(ConnectException.Kind.Unreachable, "Secure handshake failed: ${e.message}", e)
                }
                val serverFp = trust.observedFingerprint
                    ?: throw ConnectException(ConnectException.Kind.Protocol, "no server identity")
                val out = BufferedOutputStream(socket.outputStream)
                val input = DataInputStream(BufferedInputStream(socket.inputStream))
                Framing.write(
                    out,
                    Message.HelloMsg(
                        Hello(
                            protocolVersion = ProtocolInfo.VERSION,
                            minProtocolVersion = ProtocolInfo.MIN_VERSION,
                            deviceId = identity.fingerprint,
                            peerDeviceId = serverFp,
                            deviceName = app.deviceName,
                            platform = "android",
                            appVersion = app.appVersion,
                        ),
                    ),
                )
                val ack = when (val m = Framing.read(input)) {
                    is Message.HelloAckMsg -> m.ack
                    is Message.Error -> throw ConnectException(
                        if (m.error.code == "incompatible_protocol") ConnectException.Kind.Incompatible else ConnectException.Kind.Rejected,
                        m.error.message,
                    )
                    else -> throw ConnectException(ConnectException.Kind.Protocol, "unexpected ${m::class.simpleName}")
                }
                AgentSession(socket, input, out, serverFp, ack, host, port)
            } catch (t: Throwable) {
                runCatching { socket.close() }
                throw t
            }
        }
    }
}
