package dev.ccontrol.core.net

import android.util.Log
import dev.ccontrol.core.data.DeviceRepository
import dev.ccontrol.core.protocol.Action
import dev.ccontrol.core.protocol.ActionResult
import dev.ccontrol.core.protocol.AudioProtocol
import dev.ccontrol.core.protocol.RouteInfo
import dev.ccontrol.core.protocol.RouteSource
import dev.ccontrol.core.protocol.SinkInfo
import dev.ccontrol.core.security.toHex
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.security.SecureRandom
import java.util.concurrent.ConcurrentHashMap

data class RouteSpec(
    val sourceDeviceId: String,
    val source: RouteSource,
    val destDeviceId: String,
    val endpointId: String?,
    val silenceLocal: Boolean,
)

sealed interface RoutePhase {
    data object PreparingDestination : RoutePhase
    data object Starting : RoutePhase
    data object Active : RoutePhase
    data object Stopping : RoutePhase
    /** Finished; [outcome] is the agent's truthful recovery outcome. */
    data class Ended(val outcome: String?, val error: String?, val note: String?) : RoutePhase
}

data class ManagedRoute(
    val routeId: String,
    val spec: RouteSpec,
    val sourceName: String,
    val destName: String,
    val phase: RoutePhase,
    val info: RouteInfo? = null,
    val sink: SinkInfo? = null,
) {
    val live get() = phase !is RoutePhase.Ended
}

/**
 * Phone-side orchestration of an audio route between two paired computers.
 * The agents own the real state (journal, leases, rollback); the phone
 * prepares the sink, starts the source, renews both leases and stops it.
 * If the phone disappears, both agents roll back on lease expiry.
 */
class RouteOrchestrator(
    private val connections: ConnectionManager,
    private val repo: DeviceRepository,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val _routes = MutableStateFlow<List<ManagedRoute>>(emptyList())
    val routes: StateFlow<List<ManagedRoute>> = _routes.asStateFlow()
    private val loops = ConcurrentHashMap<String, Job>()
    private val rng = SecureRandom()
    private val lease = AudioProtocol.DEFAULT_LEASE_SECS

    val hasLiveRoutes: Boolean get() = _routes.value.any { it.live }

    private fun update(id: String, f: (ManagedRoute) -> ManagedRoute) =
        _routes.update { list -> list.map { if (it.routeId == id) f(it) else it } }

    private fun randomHex(n: Int) = ByteArray(n).also(rng::nextBytes).toHex()

    /** Runs the whole start sequence; returns the resulting route (active or ended). */
    suspend fun start(spec: RouteSpec): ManagedRoute {
        val src = repo.get(spec.sourceDeviceId) ?: error("unknown source computer")
        val dst = repo.get(spec.destDeviceId) ?: error("unknown destination computer")
        val id = "r-" + randomHex(6)
        val key = randomHex(32)
        _routes.update { it.filter { r -> r.live || r.routeId != id } + ManagedRoute(id, spec, src.displayName, dst.displayName, RoutePhase.PreparingDestination) }

        val sink = try {
            (connections.call(dst.deviceId, Action.SinkPrepare(id, key, spec.endpointId, lease)) as ActionResult.Sink).sink
        } catch (t: Throwable) {
            update(id) { it.copy(phase = RoutePhase.Ended(null, "${dst.displayName}: ${t.message}", null)) }
            return _routes.value.first { it.routeId == id }
        }
        update(id) { it.copy(phase = RoutePhase.Starting, sink = sink) }

        // The source reaches the destination at the address this phone uses for it (same LAN).
        val destHost = (connections.states.value[dst.deviceId] as? LinkState.Online)?.host ?: dst.lastHost
        val info = try {
            (connections.call(src.deviceId, Action.RouteStart(id, spec.source, destHost, sink.udpPort, key, lease, spec.silenceLocal), timeoutMs = 25_000) as ActionResult.Route).route
        } catch (t: Throwable) {
            runCatching { connections.call(dst.deviceId, Action.SinkStop(id)) }
            update(id) { it.copy(phase = RoutePhase.Ended(null, "${src.displayName}: ${t.message}", null)) }
            return _routes.value.first { it.routeId == id }
        }
        if (info.state != "active") {
            runCatching { connections.call(dst.deviceId, Action.SinkStop(id)) }
            update(id) { it.copy(info = info, phase = RoutePhase.Ended(info.outcome, info.error, info.note)) }
        } else {
            update(id) { it.copy(info = info, phase = RoutePhase.Active) }
            loops[id] = scope.launch { supervise(id) }
        }
        return _routes.value.first { it.routeId == id }
    }

    /** Renew leases (every lease/3) and refresh stats (every 2 s) while active. */
    private suspend fun supervise(id: String) {
        var lastRenew = 0L
        while (scope.isActive) {
            val r = _routes.value.firstOrNull { it.routeId == id } ?: return
            if (r.phase !is RoutePhase.Active) return
            val now = System.currentTimeMillis()
            if (now - lastRenew >= lease * 1000L / 3) {
                lastRenew = now
                runCatching { connections.call(r.spec.sourceDeviceId, Action.LeaseRenew(id)) }
                    .onFailure { Log.w(TAG, "source lease renew failed: ${it.message}") }
                runCatching { connections.call(r.spec.destDeviceId, Action.LeaseRenew(id)) }
                    .onFailure { Log.w(TAG, "sink lease renew failed: ${it.message}") }
            }
            val info = runCatching {
                (connections.call(r.spec.sourceDeviceId, Action.RouteList) as ActionResult.Routes).routes.firstOrNull { it.routeId == id }
            }.getOrNull()
            val sink = runCatching {
                (connections.call(r.spec.destDeviceId, Action.RouteList) as ActionResult.Routes).sinks.firstOrNull { it.routeId == id }
            }.getOrNull()
            update(id) { m ->
                val ended = info != null && info.isTerminal
                m.copy(
                    info = info ?: m.info,
                    sink = sink ?: m.sink,
                    phase = if (ended) RoutePhase.Ended(info.outcome, info.error, info.note) else m.phase,
                )
            }
            if (info != null && info.isTerminal) {
                runCatching { connections.call(r.spec.destDeviceId, Action.SinkStop(id)) }
                return
            }
            delay(2000)
        }
    }

    suspend fun stop(id: String) {
        val r = _routes.value.firstOrNull { it.routeId == id } ?: return
        if (!r.live) return
        loops.remove(id)?.cancel()
        update(id) { it.copy(phase = RoutePhase.Stopping) }
        val info = runCatching { (connections.call(r.spec.sourceDeviceId, Action.RouteStop(id), timeoutMs = 20_000) as ActionResult.Route).route }
        val sink = runCatching { (connections.call(r.spec.destDeviceId, Action.SinkStop(id)) as ActionResult.Sink).sink }.getOrNull()
        update(id) { m ->
            info.fold(
                onSuccess = { m.copy(info = it, sink = sink ?: m.sink, phase = RoutePhase.Ended(it.outcome, it.error, it.note)) },
                // The source is unreachable: its lease will expire and it rolls back on its own.
                onFailure = { m.copy(sink = sink ?: m.sink, phase = RoutePhase.Ended(null, "Source unreachable (${it.message}); it will restore itself when its lease expires.", null)) },
            )
        }
    }

    fun stopAll() {
        scope.launch { _routes.value.filter { it.live }.forEach { stop(it.routeId) } }
    }

    fun clearEnded() = _routes.update { list -> list.filter { it.live } }

    companion object { private const val TAG = "ccontrol.routes" }
}
