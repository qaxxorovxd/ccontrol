package dev.ccontrol.core.net

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.util.Log
import dev.ccontrol.core.data.DeviceRepository
import dev.ccontrol.core.data.PairedDevice
import dev.ccontrol.core.protocol.Action
import dev.ccontrol.core.protocol.ActionResult
import dev.ccontrol.core.protocol.CapabilityManifest
import dev.ccontrol.core.security.DeviceIdentity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.util.concurrent.ConcurrentHashMap
import kotlin.random.Random

/** User-visible connection state of one paired computer. */
sealed interface LinkState {
    /** App is in the background; no connection attempts (battery). */
    data object Idle : LinkState
    data object Connecting : LinkState
    data class Online(val host: String, val manifest: CapabilityManifest?, val scopes: List<String>) : LinkState
    data class Offline(val reason: String) : LinkState
    /** The computer no longer trusts this phone (revoked/reset): pairing is required again. */
    data class NotTrusted(val reason: String) : LinkState
    /** A different computer answered at the known address. */
    data class IdentityChanged(val reason: String) : LinkState
    data class Incompatible(val reason: String) : LinkState
}

class ConnectionManager(
    context: Context,
    private val repo: DeviceRepository,
    private val identity: () -> DeviceIdentity,
    private val app: AppInfo,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val discovery = Discovery(context)
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)

    private val _states = MutableStateFlow<Map<String, LinkState>>(emptyMap())
    val states: StateFlow<Map<String, LinkState>> = _states.asStateFlow()

    private val _discovered = MutableStateFlow<Map<String, DiscoveredAgent>>(emptyMap())
    val discovered: StateFlow<Map<String, DiscoveredAgent>> = _discovered.asStateFlow()

    private val sessions = ConcurrentHashMap<String, AgentSession>()
    private val supervisors = ConcurrentHashMap<String, Job>()
    private val wake = MutableSharedFlow<Unit>(extraBufferCapacity = 1, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    private var backgroundJobs = emptyList<Job>()
    @Volatile private var active = false
    @Volatile private var uiForeground = false
    @Volatile private var keepAlive = false

    private val netCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) { wake.tryEmit(Unit) }
        override fun onLost(network: Network) { wake.tryEmit(Unit) }
    }

    /** Called from ProcessLifecycleOwner: connect only while the UI is in use. */
    @Synchronized
    fun setForeground(foreground: Boolean) {
        uiForeground = foreground
        apply()
    }

    /**
     * Keep connections while the UI is in the background (an audio route is
     * active and its leases must be renewed; a foreground service is shown).
     */
    @Synchronized
    fun setKeepAlive(keep: Boolean) {
        keepAlive = keep
        apply()
    }

    @Synchronized
    private fun apply() {
        val want = uiForeground || keepAlive
        if (want == active) return
        active = want
        if (want) start() else stop()
    }

    private fun start() {
        runCatching { connectivity.registerDefaultNetworkCallback(netCallback) }
        backgroundJobs = listOf(
            scope.launch {
                repo.devices.collect { list ->
                    val ids = list.map { it.deviceId }.toSet()
                    ids.forEach(::ensureSupervisor)
                    (supervisors.keys - ids).forEach { stopDevice(it) }
                }
            },
            scope.launch {
                runCatching {
                    discovery.browse().collect { agents ->
                        val before = _discovered.value
                        _discovered.value = agents.associateBy { it.deviceId }
                        if (agents.any { before[it.deviceId]?.host != it.host }) wake.tryEmit(Unit)
                    }
                }.onFailure { Log.w(TAG, "discovery unavailable", it) }
            },
        )
    }

    private fun stop() {
        runCatching { connectivity.unregisterNetworkCallback(netCallback) }
        backgroundJobs.forEach { it.cancel() }
        supervisors.values.forEach { it.cancel() }
        supervisors.clear()
        sessions.values.forEach { it.close() }
        sessions.clear()
        _discovered.value = emptyMap()
        _states.update { m -> m.mapValues { LinkState.Idle } }
    }

    private fun setState(id: String, s: LinkState) = _states.update { it + (id to s) }

    private fun ensureSupervisor(id: String) {
        if (!active || supervisors[id]?.isActive == true) return
        supervisors[id] = scope.launch { supervise(id) }
    }

    private fun stopDevice(id: String) {
        supervisors.remove(id)?.cancel()
        sessions.remove(id)?.close()
        _states.update { it - id }
    }

    /** Wake every supervisor now (pull-to-refresh, network change). */
    fun reconnectNow() { wake.tryEmit(Unit) }

    private suspend fun waitBackoff(attempt: Int) {
        val base = (1000L shl attempt.coerceAtMost(5)).coerceAtMost(30_000L)
        val jitter = Random.nextLong(0, base / 3 + 1)
        withTimeoutOrNull(base + jitter) { wake.first() }
    }

    private fun candidates(d: PairedDevice): List<Pair<String, Int>> =
        listOfNotNull(_discovered.value[d.deviceId]?.let { it.host to it.port }, d.lastHost to d.port).distinct()

    private suspend fun supervise(id: String) {
        var attempt = 0
        while (scope.isActive && active) {
            val device = repo.get(id) ?: return
            setState(id, LinkState.Connecting)
            var lastError: LinkState = LinkState.Offline("Not reachable")
            var session: AgentSession? = null
            for ((host, port) in candidates(device)) {
                try {
                    val s = AgentSession.open(host, port, identity(), device.deviceId, app)
                    if (!s.helloAck.paired) {
                        s.close()
                        lastError = LinkState.NotTrusted("${device.displayName} no longer trusts this phone. Pair again to restore control.")
                        continue
                    }
                    session = s
                    break
                } catch (e: ConnectException) {
                    lastError = when (e.kind) {
                        ConnectException.Kind.IdentityChanged -> LinkState.IdentityChanged(e.message ?: "")
                        ConnectException.Kind.Incompatible -> LinkState.Incompatible(e.message ?: "")
                        ConnectException.Kind.Rejected -> LinkState.NotTrusted(e.message ?: "")
                        else -> LinkState.Offline(e.message ?: "Not reachable")
                    }
                } catch (e: Exception) {
                    lastError = LinkState.Offline(e.message ?: e.javaClass.simpleName)
                }
            }
            if (session == null) {
                setState(id, lastError)
                // Trust problems need the user; don't hammer the computer.
                waitBackoff(if (lastError is LinkState.Offline) attempt++ else 5)
                continue
            }
            attempt = 0
            runSession(device, session)
        }
    }

    private suspend fun runSession(device: PairedDevice, s: AgentSession) {
        val id = device.deviceId
        sessions[id] = s
        s.startReader()
        val ack = s.helloAck
        repo.update(id) {
            it.copy(
                name = ack.deviceName,
                lastHost = s.host,
                port = s.port,
                lastSeen = System.currentTimeMillis(),
                grantedScopes = ack.grantedScopes,
                manifest = ack.manifest ?: it.manifest,
                osFamily = ack.manifest?.osFamily ?: it.osFamily,
            )
        }
        setState(id, LinkState.Online(s.host, ack.manifest, ack.grantedScopes))
        val pinger = scope.launch {
            var seq = 0L
            while (isActive) {
                delay(30_000)
                runCatching { s.ping(++seq) }
            }
        }
        val events = scope.launch {
            s.events.collect { e ->
                if (e.event == "revoked") setState(id, LinkState.NotTrusted("Access was revoked on ${device.displayName}."))
            }
        }
        val failure = s.closed.await()
        pinger.cancel(); events.cancel()
        sessions.remove(id, s)
        if (_states.value[id] !is LinkState.NotTrusted) {
            setState(id, LinkState.Offline(failure?.message ?: "Connection closed"))
        } else {
            waitBackoff(5)
        }
    }

    suspend fun call(id: String, action: Action, timeoutMs: Long = 12_000): ActionResult {
        val s = sessions[id] ?: throw IllegalStateException("Computer is offline")
        return s.request(action, timeoutMs)
    }

    fun isOnline(id: String) = sessions[id]?.isOpen == true

    /** Record a freshly paired computer and connect to it immediately. */
    fun adopt(device: PairedDevice, session: AgentSession) {
        repo.upsert(device)
        if (!active) {
            session.close()
            return
        }
        supervisors.remove(device.deviceId)?.cancel()
        supervisors[device.deviceId] = scope.launch {
            runSession(device, session)
            supervise(device.deviceId)
        }
    }

    fun forget(id: String) {
        stopDevice(id)
        repo.remove(id)
    }

    suspend fun shutdown() {
        setForeground(false)
        scope.coroutineContext[Job]?.cancelAndJoin()
    }

    companion object { private const val TAG = "ccontrol" }
}
