package dev.ccontrol.core.protocol

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** Mirrors `crates/protocol/src/audio.rs`. */
object AudioProtocol {
    const val DEFAULT_LEASE_SECS = 30
}

@Serializable
data class AudioEndpoint(val id: String, val name: String, @SerialName("is_default") val isDefault: Boolean = false)

@Serializable
data class AudioSession(
    @SerialName("session_id") val sessionId: String,
    val pid: Long,
    @SerialName("app_name") val appName: String,
    @SerialName("display_name") val displayName: String? = null,
    val active: Boolean = false,
    val peak: Float = 0f,
    val muted: Boolean = false,
    val volume: Float = 1f,
    @SerialName("system_sounds") val systemSounds: Boolean = false,
    @SerialName("process_capture") val processCapture: Boolean = false,
    @SerialName("process_capture_reason") val processCaptureReason: String? = null,
    @SerialName("can_silence_local") val canSilenceLocal: Boolean = false,
)

@Serializable
data class AudioInventory(
    val endpoints: List<AudioEndpoint> = emptyList(),
    val sessions: List<AudioSession> = emptyList(),
    @SerialName("endpoint_capture") val endpointCapture: Boolean = false,
    @SerialName("process_capture") val processCapture: Boolean = false,
    @SerialName("process_capture_reason") val processCaptureReason: String? = null,
    @SerialName("can_play") val canPlay: Boolean = false,
)

/** What to route. Encoded as {"kind": ...}. */
sealed class RouteSource {
    data class Session(val pid: Long, val appName: String?) : RouteSource()
    data object Output : RouteSource()
    data object TestTone : RouteSource()

    fun toJson(): JsonObject = when (this) {
        is Session -> JsonObject(
            buildMap {
                put("kind", JsonPrimitive("session"))
                put("pid", JsonPrimitive(pid))
                appName?.let { put("app_name", JsonPrimitive(it)) }
            },
        )
        Output -> JsonObject(mapOf("kind" to JsonPrimitive("endpoint")))
        TestTone -> JsonObject(mapOf("kind" to JsonPrimitive("test_tone")))
    }

    fun label(): String = when (this) {
        is Session -> appName ?: "PID $pid"
        Output -> "Everything playing"
        TestTone -> "Test tone"
    }
}

@Serializable
data class StreamStats(
    val packets: Long = 0,
    val lost: Long = 0,
    val late: Long = 0,
    val duplicates: Long = 0,
    @SerialName("concealed_frames") val concealedFrames: Long = 0,
    @SerialName("frames_played") val framesPlayed: Long = 0,
    val underruns: Long = 0,
    @SerialName("jitter_ms") val jitterMs: Float = 0f,
    @SerialName("buffer_ms") val bufferMs: Float = 0f,
    @SerialName("rtt_ms") val rttMs: Float? = null,
    @SerialName("latency_ms") val latencyMs: Float? = null,
    @SerialName("bitrate_kbps") val bitrateKbps: Float = 0f,
    val peak: Float = 0f,
)

@Serializable
data class RouteInfo(
    @SerialName("route_id") val routeId: String,
    /** validating | starting_stream | verifying_playback | applying_reversible_source_change | active | stopping | restoring_source | rolling_back | idle | recovery_required */
    val state: String,
    val source: JsonObject,
    /** process | endpoint | test_tone */
    val capture: String,
    val dest: String,
    val owner: String,
    @SerialName("silence_local_requested") val silenceLocalRequested: Boolean = false,
    @SerialName("local_silenced") val localSilenced: Boolean = false,
    @SerialName("lease_remaining_ms") val leaseRemainingMs: Long = 0,
    /** restored | not_needed | left_user_change | partial | manual_action_required */
    val outcome: String? = null,
    val note: String? = null,
    val error: String? = null,
    val stats: StreamStats = StreamStats(),
) {
    val isTerminal get() = state == "idle" || state == "recovery_required"
}

@Serializable
data class SinkInfo(
    @SerialName("route_id") val routeId: String,
    /** waiting | playing | stopped | failed */
    val state: String,
    @SerialName("udp_port") val udpPort: Int,
    @SerialName("endpoint_name") val endpointName: String,
    val owner: String,
    @SerialName("lease_remaining_ms") val leaseRemainingMs: Long = 0,
    val error: String? = null,
    val stats: StreamStats = StreamStats(),
)

fun outcomeText(o: String?): String = when (o) {
    "restored" -> "Original playback restored"
    "not_needed" -> "Nothing to restore"
    "left_user_change" -> "Left as you changed it on the computer"
    "partial" -> "Partially restored (the app's audio session changed)"
    "manual_action_required" -> "Could not restore automatically: unmute the app on the computer"
    else -> ""
}

fun routeStateText(s: String): String = when (s) {
    "validating" -> "Checking source"
    "starting_stream" -> "Starting stream"
    "verifying_playback" -> "Verifying playback"
    "applying_reversible_source_change" -> "Silencing locally"
    "active" -> "Active"
    "stopping" -> "Stopping"
    "restoring_source" -> "Restoring source"
    "rolling_back" -> "Rolling back"
    "idle" -> "Stopped"
    "recovery_required" -> "Needs attention"
    else -> s
}
