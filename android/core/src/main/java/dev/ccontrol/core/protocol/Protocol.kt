package dev.ccontrol.core.protocol

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.encodeToJsonElement
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

/**
 * Kotlin mirror of `crates/protocol`. The JSON shapes are pinned by
 * `ProtocolTest` against literals copied from the Rust tests, so both sides
 * stay wire-compatible.
 */
object ProtocolInfo {
    const val VERSION = 1
    const val MIN_VERSION = 1
    const val DEFAULT_PORT = 47800
    const val MDNS_TYPE = "_ccontrol._tcp."
    const val MAX_FRAME = 1024 * 1024
}

object Scopes {
    const val DEVICE_READ = "device.read"
    const val TELEMETRY_READ = "telemetry.read"
    const val NOTIFICATIONS_READ = "notifications.read"
    const val MEDIA_CONTROL = "media.control"
    const val AUDIO_ROUTE = "audio.route"
    const val INPUT_CONTROL = "input.control"
    const val FILES_READ = "files.read"
    const val FILES_WRITE = "files.write"
    const val PROCESS_TERMINATE = "process.terminate"
    const val POWER_CONTROL = "power.control"
    const val COMMANDS_RUN = "commands.run"
    const val ADMIN_ACTION = "admin.action"

    /** What the phone asks for by default; the desktop owner decides what to grant. */
    val DEFAULT_REQUEST = listOf(DEVICE_READ, TELEMETRY_READ, MEDIA_CONTROL)
}

@Serializable
data class CapabilityStatus(
    val available: Boolean,
    val reason: String? = null,
    val scope: String? = null,
)

@Serializable
data class CapabilityManifest(
    @SerialName("device_id") val deviceId: String,
    @SerialName("device_name") val deviceName: String,
    @SerialName("os_family") val osFamily: String,
    @SerialName("os_distribution") val osDistribution: String? = null,
    @SerialName("os_version") val osVersion: String,
    @SerialName("os_build") val osBuild: String? = null,
    @SerialName("kernel_version") val kernelVersion: String? = null,
    val architecture: String,
    @SerialName("session_type") val sessionType: String,
    @SerialName("agent_version") val agentVersion: String,
    @SerialName("protocol_version") val protocolVersion: Int,
    val capabilities: Map<String, CapabilityStatus> = emptyMap(),
)

@Serializable
data class HelloAck(
    @SerialName("protocol_version") val protocolVersion: Int,
    @SerialName("device_id") val deviceId: String,
    @SerialName("device_name") val deviceName: String,
    val paired: Boolean,
    @SerialName("granted_scopes") val grantedScopes: List<String> = emptyList(),
    val manifest: CapabilityManifest? = null,
)

@Serializable
data class Hello(
    @SerialName("protocol_version") val protocolVersion: Int,
    @SerialName("min_protocol_version") val minProtocolVersion: Int,
    @SerialName("device_id") val deviceId: String,
    @SerialName("peer_device_id") val peerDeviceId: String,
    @SerialName("device_name") val deviceName: String,
    val platform: String,
    @SerialName("app_version") val appVersion: String,
)

@Serializable
data class ProtocolError(val code: String, val message: String)

@Serializable
data class VolumeState(
    val level: Float,
    val muted: Boolean,
    @SerialName("endpoint_name") val endpointName: String? = null,
)

@Serializable
data class Telemetry(
    @SerialName("cpu_percent") val cpuPercent: Float,
    @SerialName("cpu_count") val cpuCount: Int,
    @SerialName("memory_used_bytes") val memoryUsedBytes: Long,
    @SerialName("memory_total_bytes") val memoryTotalBytes: Long,
    @SerialName("uptime_secs") val uptimeSecs: Long,
    @SerialName("battery_percent") val batteryPercent: Float? = null,
    @SerialName("battery_charging") val batteryCharging: Boolean? = null,
    val disks: List<DiskInfo> = emptyList(),
)

@Serializable
data class DiskInfo(
    val mount: String,
    val name: String,
    @SerialName("total_bytes") val totalBytes: Long,
    @SerialName("available_bytes") val availableBytes: Long,
    val removable: Boolean = false,
)

@Serializable
data class MediaStatus(
    val active: Boolean,
    val app: String? = null,
    val title: String? = null,
    val artist: String? = null,
    val album: String? = null,
    /** playing | paused | stopped | unknown */
    val status: String = "unknown",
    @SerialName("position_ms") val positionMs: Long? = null,
    @SerialName("duration_ms") val durationMs: Long? = null,
    @SerialName("can_play_pause") val canPlayPause: Boolean = false,
    @SerialName("can_next") val canNext: Boolean = false,
    @SerialName("can_previous") val canPrevious: Boolean = false,
) {
    val isPlaying get() = status == "playing"
}

@Serializable
data class ProcessInfo(
    val pid: Long,
    @SerialName("parent_pid") val parentPid: Long? = null,
    val name: String,
    val exe: String? = null,
    @SerialName("cpu_percent") val cpuPercent: Float,
    @SerialName("memory_bytes") val memoryBytes: Long,
    val status: String = "",
    @SerialName("start_time") val startTime: Long = 0,
    @SerialName("same_user") val sameUser: Boolean = false,
    val terminable: Boolean = false,
    @SerialName("protected_reason") val protectedReason: String? = null,
)

@Serializable
data class TerminateOutcome(
    val pid: Long,
    /** graceful | force */
    val method: String,
    val exited: Boolean,
    val note: String? = null,
)

@Serializable
data class AppWindow(
    @SerialName("window_id") val windowId: Long,
    val pid: Long,
    @SerialName("process_name") val processName: String,
    val title: String,
    val focused: Boolean = false,
    val minimized: Boolean = false,
)

enum class MediaKey(val wire: String) { PlayPause("play_pause"), Next("next"), Previous("previous"), Stop("stop") }

/** Typed logical actions; never shell strings. */
sealed class Action(val name: String) {
    data object DeviceInfo : Action("device.info")
    data object TelemetryGet : Action("telemetry.get")
    data object VolumeGet : Action("volume.get")
    data class VolumeSet(val level: Float) : Action("volume.set") {
        init { require(level.isFinite() && level in 0f..1f) { "level must be within 0..1" } }
    }
    data class VolumeMute(val muted: Boolean) : Action("volume.mute")
    data class Media(val key: MediaKey) : Action("media.key")
    data object MediaStatusGet : Action("media.status")
    data object ProcessList : Action("process.list")
    data class ProcessTerminate(val pid: Long, val processName: String, val force: Boolean) : Action("process.terminate")
    data object AppList : Action("app.list")
    data class AppClose(val windowId: Long, val pid: Long) : Action("app.close")
    data object Unpair : Action("pairing.unpair")
    data object AudioDevices : Action("audio.devices")
    data class SinkPrepare(val routeId: String, val key: String, val endpointId: String?, val leaseSecs: Int) : Action("audio.sink.prepare")
    data class SinkStop(val routeId: String) : Action("audio.sink.stop")
    data class RouteStart(
        val routeId: String,
        val source: RouteSource,
        val destHost: String,
        val destPort: Int,
        val key: String,
        val leaseSecs: Int,
        val silenceLocal: Boolean,
    ) : Action("route.audio.start")
    data class RouteStop(val routeId: String) : Action("route.audio.stop")
    data class LeaseRenew(val routeId: String) : Action("route.lease.renew")
    data object RouteList : Action("route.list")
}

sealed interface ActionResult {
    data class Manifest(val manifest: CapabilityManifest) : ActionResult
    data class TelemetryResult(val telemetry: Telemetry) : ActionResult
    data class Volume(val state: VolumeState) : ActionResult
    data class Media(val status: MediaStatus) : ActionResult
    data class Processes(val processes: List<ProcessInfo>) : ActionResult
    data class Terminated(val outcome: TerminateOutcome) : ActionResult
    data class Apps(val windows: List<AppWindow>) : ActionResult
    data class Audio(val inventory: AudioInventory) : ActionResult
    data class Sink(val sink: SinkInfo) : ActionResult
    data class Route(val route: RouteInfo) : ActionResult
    data class Routes(val routes: List<RouteInfo>, val sinks: List<SinkInfo>) : ActionResult
    data object Done : ActionResult
    data class Unknown(val kind: String) : ActionResult
}

sealed interface Message {
    data class HelloMsg(val hello: Hello) : Message
    data class HelloAckMsg(val ack: HelloAck) : Message
    data class PairCommit(val commitment: String, val requestedScopes: List<String>) : Message
    data class PairNonce(val nonce: String) : Message
    data class PairReveal(val nonce: String) : Message
    data class PairAwaitingApproval(val timeoutSecs: Int) : Message
    data class PairResult(val accepted: Boolean, val grantedScopes: List<String>, val reason: String?) : Message
    data class Request(val id: Long, val action: Action) : Message
    data class Response(val id: Long, val result: ActionResult?, val error: ProtocolError?) : Message
    data class Event(val event: String, val body: JsonObject) : Message
    data class Ping(val seq: Long) : Message
    data class Pong(val seq: Long) : Message
    data class Error(val error: ProtocolError) : Message
}

class ProtocolException(message: String) : Exception(message)

object Wire {
    val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        encodeDefaults = false
    }

    fun encode(m: Message): String = json.encodeToString(JsonObject.serializer(), toJson(m))

    fun toJson(m: Message): JsonObject = when (m) {
        is Message.HelloMsg -> withType("hello", json.encodeToJsonElement(m.hello).jsonObject)
        is Message.HelloAckMsg -> withType("hello_ack", json.encodeToJsonElement(m.ack).jsonObject)
        is Message.PairCommit -> buildJsonObject {
            put("type", "pair_commit")
            put("commitment", m.commitment)
            put("requested_scopes", json.encodeToJsonElement(m.requestedScopes))
        }
        is Message.PairNonce -> buildJsonObject { put("type", "pair_nonce"); put("nonce", m.nonce) }
        is Message.PairReveal -> buildJsonObject { put("type", "pair_reveal"); put("nonce", m.nonce) }
        is Message.PairAwaitingApproval -> buildJsonObject { put("type", "pair_awaiting_approval"); put("timeout_secs", m.timeoutSecs) }
        is Message.PairResult -> buildJsonObject {
            put("type", "pair_result"); put("accepted", m.accepted)
            put("granted_scopes", json.encodeToJsonElement(m.grantedScopes))
            m.reason?.let { put("reason", it) }
        }
        is Message.Request -> buildJsonObject {
            put("type", "request")
            put("id", m.id)
            put("action", m.action.name)
            when (val a = m.action) {
                is Action.VolumeSet -> put("level", a.level)
                is Action.VolumeMute -> put("muted", a.muted)
                is Action.Media -> put("key", a.key.wire)
                is Action.ProcessTerminate -> { put("pid", a.pid); put("name", a.processName); put("force", a.force) }
                is Action.AppClose -> { put("window_id", a.windowId); put("pid", a.pid) }
                is Action.SinkPrepare -> {
                    put("route_id", a.routeId); put("key", a.key)
                    a.endpointId?.let { put("endpoint_id", it) }
                    put("lease_secs", a.leaseSecs)
                }
                is Action.SinkStop -> put("route_id", a.routeId)
                is Action.RouteStart -> {
                    put("route_id", a.routeId); put("source", a.source.toJson())
                    put("dest_host", a.destHost); put("dest_port", a.destPort)
                    put("key", a.key); put("lease_secs", a.leaseSecs); put("silence_local", a.silenceLocal)
                }
                is Action.RouteStop -> put("route_id", a.routeId)
                is Action.LeaseRenew -> put("route_id", a.routeId)
                else -> {}
            }
        }
        is Message.Ping -> buildJsonObject { put("type", "ping"); put("seq", m.seq) }
        is Message.Pong -> buildJsonObject { put("type", "pong"); put("seq", m.seq) }
        is Message.Error -> buildJsonObject { put("type", "error"); put("error", json.encodeToJsonElement(m.error)) }
        is Message.Response, is Message.Event -> throw ProtocolException("controller never sends ${m::class.simpleName}")
    }

    private fun withType(type: String, o: JsonObject) = JsonObject(mapOf("type" to JsonPrimitive(type)) + o)

    fun decode(text: String): Message {
        val o = try {
            json.parseToJsonElement(text).jsonObject
        } catch (e: Exception) {
            throw ProtocolException("malformed frame: ${e.message}")
        }
        val type = o.str("type") ?: throw ProtocolException("missing type")
        return when (type) {
            "hello_ack" -> Message.HelloAckMsg(json.decodeFromJsonElement(o))
            "pair_nonce" -> Message.PairNonce(o.str("nonce") ?: throw ProtocolException("nonce"))
            "pair_awaiting_approval" -> Message.PairAwaitingApproval(o["timeout_secs"]?.jsonPrimitive?.longOrNull?.toInt() ?: 120)
            "pair_result" -> Message.PairResult(
                accepted = o["accepted"]?.jsonPrimitive?.booleanOrNull ?: false,
                grantedScopes = o["granted_scopes"]?.let { json.decodeFromJsonElement<List<String>>(it) } ?: emptyList(),
                reason = o.str("reason"),
            )
            "response" -> Message.Response(
                id = o["id"]?.jsonPrimitive?.longOrNull ?: throw ProtocolException("id"),
                result = (o["result"] as? JsonObject)?.let(::decodeResult),
                error = o["error"]?.let { json.decodeFromJsonElement<ProtocolError>(it) },
            )
            "event" -> Message.Event(o.str("event") ?: "unknown", o)
            "ping" -> Message.Ping(o["seq"]?.jsonPrimitive?.longOrNull ?: 0)
            "pong" -> Message.Pong(o["seq"]?.jsonPrimitive?.longOrNull ?: 0)
            "error" -> Message.Error(json.decodeFromJsonElement(o["error"] ?: throw ProtocolException("error body")))
            else -> throw ProtocolException("unexpected message type '$type'")
        }
    }

    fun decodeResult(o: JsonObject): ActionResult = when (val kind = o.str("kind")) {
        "manifest" -> ActionResult.Manifest(json.decodeFromJsonElement(o["manifest"] ?: throw ProtocolException("manifest")))
        "telemetry" -> ActionResult.TelemetryResult(json.decodeFromJsonElement(o))
        "volume" -> ActionResult.Volume(json.decodeFromJsonElement(o))
        "media_status" -> ActionResult.Media(json.decodeFromJsonElement(o))
        "processes" -> ActionResult.Processes(json.decodeFromJsonElement(o["processes"] ?: throw ProtocolException("processes")))
        "terminated" -> ActionResult.Terminated(json.decodeFromJsonElement(o))
        "apps" -> ActionResult.Apps(json.decodeFromJsonElement(o["windows"] ?: throw ProtocolException("windows")))
        "audio" -> ActionResult.Audio(json.decodeFromJsonElement(o))
        "sink" -> ActionResult.Sink(json.decodeFromJsonElement(o))
        "route" -> ActionResult.Route(json.decodeFromJsonElement(o))
        "routes" -> ActionResult.Routes(
            json.decodeFromJsonElement(o["routes"] ?: throw ProtocolException("routes")),
            json.decodeFromJsonElement(o["sinks"] ?: throw ProtocolException("sinks")),
        )
        "done" -> ActionResult.Done
        else -> ActionResult.Unknown(kind ?: "?")
    }

    private fun JsonObject.str(k: String): String? = (this[k] as? JsonPrimitive)?.takeIf { it.isString }?.content

    fun JsonElement.asVolume(): VolumeState = json.decodeFromJsonElement(this)
}
