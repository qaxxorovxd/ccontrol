package dev.ccontrol.app.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.PushPin
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.ccontrol.app.CControlApp
import dev.ccontrol.app.ui.components.OsBadge
import dev.ccontrol.app.ui.components.Panel
import dev.ccontrol.app.ui.components.SectionLabel
import dev.ccontrol.app.ui.components.StatusPill
import dev.ccontrol.app.ui.components.formatBytes
import dev.ccontrol.app.ui.components.formatUptime
import dev.ccontrol.app.ui.theme.Mono
import dev.ccontrol.app.ui.theme.Palette
import dev.ccontrol.core.net.LinkState
import dev.ccontrol.core.net.RemoteException
import dev.ccontrol.core.protocol.Action
import dev.ccontrol.core.protocol.ActionResult
import dev.ccontrol.core.protocol.MediaKey
import dev.ccontrol.core.protocol.MediaStatus
import androidx.compose.foundation.clickable
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Pause
import androidx.compose.ui.text.style.TextOverflow
import dev.ccontrol.core.protocol.Scopes
import dev.ccontrol.core.protocol.Telemetry
import dev.ccontrol.core.protocol.VolumeState
import dev.ccontrol.core.security.Fingerprint
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

internal fun Throwable.userMessage(): String = when (this) {
    is RemoteException -> when (error.code) {
        "scope_denied" -> "Not permitted: ${error.message}"
        "forbidden" -> "Protected: ${error.message}"
        "unsupported" -> "Not supported on this computer: ${error.message}"
        else -> error.message
    }
    else -> message ?: javaClass.simpleName
}

@Composable
fun DeviceScreen(
    app: CControlApp,
    deviceId: String,
    onBack: () -> Unit,
    onOpenProcesses: () -> Unit = {},
    onOpenApps: () -> Unit = {},
) {
    val devices by app.repo.devices.collectAsStateWithLifecycle()
    val states by app.connections.states.collectAsStateWithLifecycle()
    val device = devices.firstOrNull { it.deviceId == deviceId }
    if (device == null) {
        LaunchedEffect(Unit) { onBack() }
        return
    }
    val state = states[deviceId]
    val online = state as? LinkState.Online
    val manifest = online?.manifest ?: device.manifest
    val scopes = online?.scopes ?: device.grantedScopes
    val scope = rememberCoroutineScope()

    var volume by remember { mutableStateOf<VolumeState?>(null) }
    var dragLevel by remember { mutableStateOf<Float?>(null) }
    var telemetry by remember { mutableStateOf<Telemetry?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var sendJob by remember { mutableStateOf<Job?>(null) }
    var confirmForget by remember { mutableStateOf(false) }
    var media by remember { mutableStateOf<MediaStatus?>(null) }
    var renaming by remember { mutableStateOf(false) }

    suspend fun call(a: Action): ActionResult? = try {
        app.connections.call(deviceId, a).also { error = null }
    } catch (t: Throwable) {
        error = t.userMessage(); null
    }

    val canMedia = Scopes.MEDIA_CONTROL in scopes
    val canTelemetry = Scopes.TELEMETRY_READ in scopes
    val volumeCap = manifest?.capabilities?.get("audio.master_volume")
    val mediaCap = manifest?.capabilities?.get("media.keys")

    LaunchedEffect(online != null, canMedia) {
        if (online != null && canMedia && volumeCap?.available == true) {
            (call(Action.VolumeGet) as? ActionResult.Volume)?.let { volume = it.state }
        }
    }
    // Telemetry refreshes only while this screen is visible and the computer is online.
    val mediaStatusCap = manifest?.capabilities?.get("media.status")
    // Now playing refreshes every 2 s while visible.
    LaunchedEffect(online != null, canMedia, mediaStatusCap?.available) {
        while (isActive && online != null && canMedia && mediaStatusCap?.available == true) {
            (runCatching { app.connections.call(deviceId, Action.MediaStatusGet) }.getOrNull() as? ActionResult.Media)?.let { media = it.status }
            delay(2000)
        }
    }
    LaunchedEffect(online != null, canTelemetry) {
        while (isActive && online != null && canTelemetry) {
            (runCatching { app.connections.call(deviceId, Action.TelemetryGet) }.getOrNull() as? ActionResult.TelemetryResult)?.let { telemetry = it.telemetry }
            delay(3000)
        }
    }

    Column(Modifier.fillMaxSize().safeDrawingPadding().verticalScroll(rememberScrollState()).padding(16.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
            OsBadge(device.osFamily)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(device.displayName, style = MaterialTheme.typography.titleMedium)
                Text(
                    listOfNotNull(manifest?.osVersion, manifest?.architecture).joinToString(" · ").ifEmpty { device.lastHost },
                    style = MaterialTheme.typography.bodySmall, color = Palette.Muted,
                )
            }
            IconButton(onClick = { app.repo.update(deviceId) { it.copy(pinned = !it.pinned) } }) {
                Icon(if (device.pinned) Icons.Filled.PushPin else Icons.Outlined.PushPin, if (device.pinned) "Unpin" else "Pin", tint = Palette.Muted)
            }
        }
        Row(Modifier.padding(top = 12.dp, start = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            StatusPill(state)
            Spacer(Modifier.width(8.dp))
            Text(
                when (state) {
                    is LinkState.Online -> state.host
                    is LinkState.Offline -> state.reason
                    is LinkState.NotTrusted -> state.reason
                    is LinkState.IdentityChanged -> state.reason
                    is LinkState.Incompatible -> state.reason
                    else -> ""
                },
                style = MaterialTheme.typography.bodySmall, color = Palette.Muted, maxLines = 2,
            )
        }
        if (state is LinkState.Offline) {
            TextButton(onClick = app.connections::reconnectNow) { Text("Retry now") }
        }
        error?.let { Text(it, color = Palette.Warn, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 8.dp, start = 4.dp)) }

        SectionLabel("Sound")
        Panel {
            when {
                !canMedia -> Disabled("Volume control was not granted (media.control). Change permissions on the computer.")
                volumeCap?.available == false -> Disabled(volumeCap.reason ?: "Unavailable")
                online == null -> Disabled("Available when the computer is online.")
                volume == null -> LinearProgressIndicator(Modifier.fillMaxWidth(), color = Palette.Violet)
                else -> {
                    val v = volume!!
                    val shown = dragLevel ?: v.level
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        FilledTonalIconButton(onClick = {
                            scope.launch { (call(Action.VolumeMute(!v.muted)) as? ActionResult.Volume)?.let { volume = it.state } }
                        }) {
                            Icon(if (v.muted) Icons.AutoMirrored.Filled.VolumeOff else Icons.AutoMirrored.Filled.VolumeUp, if (v.muted) "Unmute" else "Mute")
                        }
                        Spacer(Modifier.width(8.dp))
                        Slider(
                            value = shown,
                            onValueChange = { lvl ->
                                dragLevel = lvl
                                // Throttle: at most one in-flight volume request while dragging.
                                if (sendJob?.isActive != true) sendJob = scope.launch {
                                    (call(Action.VolumeSet(lvl)) as? ActionResult.Volume)?.let { volume = it.state }
                                    delay(80)
                                }
                            },
                            onValueChangeFinished = {
                                val final = dragLevel ?: return@Slider
                                scope.launch {
                                    sendJob?.join()
                                    (call(Action.VolumeSet(final)) as? ActionResult.Volume)?.let { volume = it.state }
                                    dragLevel = null
                                }
                            },
                            modifier = Modifier.weight(1f).semantics { stateDescription = "${(shown * 100).toInt()} percent" },
                            colors = SliderDefaults.colors(thumbColor = Palette.Violet, activeTrackColor = Palette.Violet),
                        )
                        Text("${(shown * 100).toInt()}%", fontFamily = Mono, modifier = Modifier.width(48.dp), color = Palette.Muted)
                    }
                    v.endpointName?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Palette.Muted) }
                }
            }
        }

        SectionLabel("Media")
        Panel {
            when {
                !canMedia -> Disabled("Media control was not granted.")
                mediaCap?.available == false -> Disabled(mediaCap.reason ?: "Unavailable")
                online == null -> Disabled("Available when the computer is online.")
                else -> {
                    NowPlaying(media)
                    val m = media
                    // Without status info (or with an active session) enable what the OS allows.
                    val known = m?.active == true
                    Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
                        MediaButton(Icons.Filled.SkipPrevious, "Previous track", enabled = !known || m.canPrevious) {
                            scope.launch { call(Action.Media(MediaKey.Previous)); media = refreshMedia(app, deviceId) ?: media }
                        }
                        MediaButton(
                            if (m?.isPlaying == true) Icons.Filled.Pause else Icons.Filled.PlayArrow,
                            if (m?.isPlaying == true) "Pause" else "Play",
                            enabled = !known || m.canPlayPause,
                        ) { scope.launch { call(Action.Media(MediaKey.PlayPause)); media = refreshMedia(app, deviceId) ?: media } }
                        MediaButton(Icons.Filled.SkipNext, "Next track", enabled = !known || m.canNext) {
                            scope.launch { call(Action.Media(MediaKey.Next)); media = refreshMedia(app, deviceId) ?: media }
                        }
                    }
                    mediaCap?.reason?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Palette.Muted, modifier = Modifier.padding(top = 8.dp)) }
                }
            }
        }

        SectionLabel("System")
        Panel {
            val t = telemetry
            when {
                !canTelemetry -> Disabled("Telemetry was not granted (telemetry.read).")
                online == null -> Disabled("Available when the computer is online.")
                t == null -> LinearProgressIndicator(Modifier.fillMaxWidth(), color = Palette.Cyan)
                else -> {
                    Meter("CPU", t.cpuPercent / 100f, "%.0f%% · %d cores".format(t.cpuPercent, t.cpuCount))
                    Meter("Memory", t.memoryUsedBytes.toFloat() / t.memoryTotalBytes.coerceAtLeast(1), "${formatBytes(t.memoryUsedBytes)} / ${formatBytes(t.memoryTotalBytes)}")
                    t.disks.forEach { d ->
                        val used = d.totalBytes - d.availableBytes
                        Meter(
                            "Disk ${d.mount.trimEnd('\\', '/').ifEmpty { "/" }}" + if (d.removable) " (removable)" else "",
                            used.toFloat() / d.totalBytes.coerceAtLeast(1),
                            "${formatBytes(d.availableBytes)} free of ${formatBytes(d.totalBytes)}",
                        )
                    }
                    Text(
                        listOfNotNull(
                            "Up ${formatUptime(t.uptimeSecs)}",
                            t.batteryPercent?.let { "Battery ${it.toInt()}%" + when (t.batteryCharging) { true -> " · plugged in"; false -> " · on battery"; null -> "" } },
                        ).joinToString("  ·  "),
                        style = MaterialTheme.typography.bodySmall, color = Palette.Muted, modifier = Modifier.padding(top = 6.dp),
                    )
                }
            }
        }

        SectionLabel("Apps")
        Panel {
            val canRead = Scopes.DEVICE_READ in scopes
            val winCap = manifest?.capabilities?.get("apps.windows")
            NavRow(
                "Apps & windows",
                if (winCap?.available == false) winCap.reason else "Open windows; close them like clicking X",
                enabled = canRead && online != null && winCap?.available != false,
                onClick = onOpenApps,
            )
            NavRow("Processes", "CPU and memory per process; quit or force quit", enabled = canRead && online != null, onClick = onOpenProcesses)
            if (!canRead) Disabled("Needs device.read, granted on the computer.")
        }

        if (manifest != null) {
            SectionLabel("Capabilities")
            Panel {
                manifest.capabilities.forEach { (k, v) ->
                    Row(Modifier.padding(vertical = 6.dp), verticalAlignment = Alignment.Top) {
                        Text(if (v.available) "●" else "○", color = if (v.available) Palette.Ok else Palette.Muted, modifier = Modifier.width(20.dp))
                        Column(Modifier.weight(1f)) {
                            Text(k, fontFamily = Mono, style = MaterialTheme.typography.bodySmall)
                            v.reason?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Palette.Muted) }
                        }
                    }
                }
            }
        }

        SectionLabel("This pairing")
        Panel {
            Text("Computer ID ${Fingerprint.short(deviceId)}", fontFamily = Mono, style = MaterialTheme.typography.bodySmall, color = Palette.Muted)
            Text("Permissions: ${scopes.joinToString(", ").ifEmpty { "none" }}", style = MaterialTheme.typography.bodySmall, color = Palette.Muted, modifier = Modifier.padding(top = 4.dp))
            Row {
                TextButton(onClick = { renaming = true }) { Text("Rename") }
                TextButton(onClick = { confirmForget = true }) { Text("Forget", color = Palette.Bad) }
            }
        }
    }

    if (renaming) {
        var name by remember { mutableStateOf(device.customName ?: device.name) }
        AlertDialog(
            onDismissRequest = { renaming = false },
            title = { Text("Rename") },
            text = { OutlinedTextField(name, { name = it.take(48) }, singleLine = true) },
            confirmButton = { TextButton(onClick = { app.repo.update(deviceId) { it.copy(customName = name.trim()) }; renaming = false }) { Text("Save") } },
            dismissButton = { TextButton(onClick = { renaming = false }) { Text("Cancel") } },
        )
    }
    if (confirmForget) {
        AlertDialog(
            onDismissRequest = { confirmForget = false },
            title = { Text("Forget ${device.displayName}?") },
            text = { Text("This phone will stop connecting to it and you will need to pair again. If the computer is online, this phone's access there is removed too; otherwise revoke it in the computer's ccontrol settings.") },
            confirmButton = { TextButton(onClick = {
                confirmForget = false
                scope.launch {
                    // Remove this phone's access on the computer too, when reachable.
                    if (online != null) runCatching { app.connections.call(deviceId, Action.Unpair) }
                    app.connections.forget(deviceId)
                    onBack()
                }
            }) { Text("Forget", color = Palette.Bad) } },
            dismissButton = { TextButton(onClick = { confirmForget = false }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun Disabled(reason: String) {
    Text(reason, style = MaterialTheme.typography.bodyMedium, color = Palette.Muted, modifier = Modifier.semantics { contentDescription = "Unavailable: $reason" })
}

@Composable
private fun MediaButton(icon: ImageVector, label: String, enabled: Boolean = true, onClick: () -> Unit) {
    FilledTonalIconButton(onClick = onClick, enabled = enabled, modifier = Modifier.width(64.dp)) { Icon(icon, label) }
}

private suspend fun refreshMedia(app: CControlApp, id: String): MediaStatus? {
    delay(350) // let the player apply the command
    return (runCatching { app.connections.call(id, Action.MediaStatusGet) }.getOrNull() as? ActionResult.Media)?.status
}

private fun formatMs(ms: Long): String = "%d:%02d".format(ms / 60000, (ms / 1000) % 60)

@Composable
private fun NowPlaying(m: MediaStatus?) {
    when {
        m == null -> {}
        !m.active -> Text("Nothing is playing.", style = MaterialTheme.typography.bodyMedium, color = Palette.Muted)
        else -> Column(Modifier.semantics(mergeDescendants = true) {}) {
            Text(m.title ?: "Unknown title", style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                listOfNotNull(m.artist, m.app?.let { "in $it" }).joinToString(" · ").ifEmpty { m.status },
                style = MaterialTheme.typography.bodySmall, color = Palette.Muted, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            val d = m.durationMs
            if (d != null && d > 0) {
                val pos = (m.positionMs ?: 0).coerceAtMost(d)
                LinearProgressIndicator(
                    progress = { pos.toFloat() / d },
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                    color = Palette.Violet, trackColor = Palette.SurfaceHigh,
                )
                Text("${formatMs(pos)} / ${formatMs(d)}", fontFamily = Mono, style = MaterialTheme.typography.bodySmall, color = Palette.Muted)
            }
        }
    }
}

@Composable
private fun NavRow(title: String, subtitle: String?, enabled: Boolean, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(enabled = enabled, onClick = onClick).padding(vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyMedium, color = if (enabled) Palette.Text else Palette.Muted)
            subtitle?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Palette.Muted) }
        }
        Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null, tint = Palette.Muted)
    }
}

@Composable
private fun Meter(label: String, fraction: Float, detail: String) {
    Column(Modifier.padding(vertical = 6.dp)) {
        Row {
            Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
            Text(detail, style = MaterialTheme.typography.bodySmall, color = Palette.Muted, fontFamily = Mono)
        }
        LinearProgressIndicator(
            progress = { fraction.coerceIn(0f, 1f) },
            modifier = Modifier.fillMaxWidth().padding(top = 6.dp),
            color = Palette.Cyan, trackColor = Palette.SurfaceHigh,
        )
    }
}
