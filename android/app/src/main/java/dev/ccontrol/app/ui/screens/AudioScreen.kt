package dev.ccontrol.app.ui.screens

import android.Manifest
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.ccontrol.app.CControlApp
import dev.ccontrol.app.ui.components.EmptyState
import dev.ccontrol.app.ui.components.Panel
import dev.ccontrol.app.ui.components.SectionLabel
import dev.ccontrol.app.ui.theme.Mono
import dev.ccontrol.app.ui.theme.Palette
import dev.ccontrol.core.data.PairedDevice
import dev.ccontrol.core.net.LinkState
import dev.ccontrol.core.net.ManagedRoute
import dev.ccontrol.core.net.RoutePhase
import dev.ccontrol.core.net.RouteSpec
import dev.ccontrol.core.protocol.Action
import dev.ccontrol.core.protocol.ActionResult
import dev.ccontrol.core.protocol.AudioInventory
import dev.ccontrol.core.protocol.RouteSource
import dev.ccontrol.core.protocol.Scopes
import dev.ccontrol.core.protocol.outcomeText
import dev.ccontrol.core.protocol.routeStateText
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** One routable thing on one computer. */
private data class SourceOption(
    val device: PairedDevice,
    val source: RouteSource,
    val title: String,
    val subtitle: String?,
    val enabled: Boolean,
    val disabledReason: String?,
    val canSilenceLocal: Boolean,
    val silenceReason: String?,
)

private data class Draft(val src: SourceOption, val dest: PairedDevice)

/**
 * Audio Center: live routes with measured metrics, and an accessible 2D route
 * matrix (sources × destination computers). Every cell calls the same real
 * route API as the future 3D drag-and-drop.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun AudioScreen(app: CControlApp, onBack: () -> Unit) {
    val devices by app.repo.devices.collectAsStateWithLifecycle()
    val states by app.connections.states.collectAsStateWithLifecycle()
    val routes by app.routes.routes.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val inventories = remember { mutableStateMapOf<String, Result<AudioInventory>>() }
    var draft by remember { mutableStateOf<Draft?>(null) }
    var starting by remember { mutableStateOf(false) }

    val notifPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {}

    val audioCapable = devices.filter { d ->
        val st = states[d.deviceId] as? LinkState.Online
        st != null && Scopes.AUDIO_ROUTE in st.scopes && st.manifest?.capabilities?.get("audio.route")?.available == true
    }

    // Refresh session inventories every 3 s while visible.
    LaunchedEffect(audioCapable.map { it.deviceId }) {
        while (isActive) {
            audioCapable.forEach { d ->
                inventories[d.deviceId] = runCatching { (app.connections.call(d.deviceId, Action.AudioDevices) as ActionResult.Audio).inventory }
            }
            delay(3000)
        }
    }

    Column(Modifier.fillMaxSize().safeDrawingPadding().verticalScroll(rememberScrollState()).padding(16.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
            Text("Audio", style = MaterialTheme.typography.headlineSmall)
        }

        if (routes.isNotEmpty()) {
            SectionLabel("Routes")
            routes.forEach { r -> RouteCard(r, onStop = { scope.launch { app.routes.stop(r.routeId) } }) }
            if (routes.any { !it.live }) TextButton(onClick = app.routes::clearEnded) { Text("Clear finished") }
        }

        SectionLabel("New route")
        if (audioCapable.size < 2) {
            EmptyState(
                "Two online computers needed",
                "Routing sends audio from one paired computer to another. Both must be online and have granted this phone the audio.route permission.",
            )
            return@Column
        }
        audioCapable.forEach { srcDev ->
            val inv = inventories[srcDev.deviceId]
            val dests = audioCapable.filter { it.deviceId != srcDev.deviceId && inventories[it.deviceId]?.getOrNull()?.canPlay != false }
            Panel(Modifier.padding(bottom = 10.dp)) {
                Text("From ${srcDev.displayName}", style = MaterialTheme.typography.titleMedium)
                when {
                    inv == null -> LinearProgressIndicator(Modifier.fillMaxWidth().padding(top = 8.dp), color = Palette.Violet)
                    inv.isFailure -> Text(inv.exceptionOrNull()?.userMessage() ?: "Unavailable", color = Palette.Warn, style = MaterialTheme.typography.bodySmall)
                    else -> {
                        val options = sourceOptions(srcDev, inv.getOrThrow())
                        options.forEach { opt ->
                            Column(Modifier.padding(top = 10.dp)) {
                                Text(opt.title, style = MaterialTheme.typography.bodyMedium, color = if (opt.enabled) Palette.Text else Palette.Muted)
                                (if (opt.enabled) opt.subtitle else opt.disabledReason)?.let {
                                    Text(it, style = MaterialTheme.typography.bodySmall, color = Palette.Muted)
                                }
                                if (opt.enabled) FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    dests.forEach { d ->
                                        AssistChip(
                                            onClick = { draft = Draft(opt, d) },
                                            label = { Text("→ ${d.displayName}") },
                                            modifier = Modifier.semantics { contentDescription = "Route ${opt.title} from ${srcDev.displayName} to ${d.displayName}" },
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Text(
            "This phone cannot be a source or destination yet: Android only lets apps capture other apps that opt in, and phone playback is planned.",
            style = MaterialTheme.typography.bodySmall, color = Palette.Muted, modifier = Modifier.padding(top = 8.dp),
        )
    }

    draft?.let { d ->
        val destInv = inventories[d.dest.deviceId]?.getOrNull()
        var endpoint by remember(d) { mutableStateOf(destInv?.endpoints?.firstOrNull { it.isDefault }?.id) }
        var silence by remember(d) { mutableStateOf(d.src.canSilenceLocal) }
        AlertDialog(
            onDismissRequest = { if (!starting) draft = null },
            title = { Text("${d.src.title} → ${d.dest.displayName}") },
            text = {
                Column {
                    Text("Play on", style = MaterialTheme.typography.labelSmall, color = Palette.Muted)
                    destInv?.endpoints?.forEach { e ->
                        Row(
                            Modifier.fillMaxWidth().selectable(selected = endpoint == e.id, onClick = { endpoint = e.id }, role = Role.RadioButton),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = endpoint == e.id, onClick = null)
                            Text(e.name + if (e.isDefault) " (default)" else "", style = MaterialTheme.typography.bodyMedium)
                        }
                    }
                    Spacer(Modifier.size(8.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(checked = silence, onCheckedChange = { silence = it }, enabled = d.src.canSilenceLocal)
                        Column {
                            Text("Silence it on ${d.src.device.displayName}", style = MaterialTheme.typography.bodyMedium)
                            Text(
                                if (d.src.canSilenceLocal) "Muted there only after playback is confirmed; restored when the route stops."
                                else d.src.silenceReason ?: "Not available; local playback continues.",
                                style = MaterialTheme.typography.bodySmall, color = Palette.Muted,
                            )
                        }
                    }
                    if (starting) LinearProgressIndicator(Modifier.fillMaxWidth().padding(top = 12.dp), color = Palette.Violet)
                }
            },
            confirmButton = {
                Button(
                    enabled = !starting,
                    onClick = {
                        if (Build.VERSION.SDK_INT >= 33) notifPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
                        starting = true
                        scope.launch {
                            app.routes.start(RouteSpec(d.src.device.deviceId, d.src.source, d.dest.deviceId, endpoint, silence && d.src.canSilenceLocal))
                            starting = false
                            draft = null
                        }
                    },
                    colors = ButtonDefaults.buttonColors(containerColor = Palette.Violet, contentColor = Palette.Background),
                ) { Text("Start") }
            },
            dismissButton = { if (!starting) TextButton(onClick = { draft = null }) { Text("Cancel") } },
        )
    }
}

private fun sourceOptions(dev: PairedDevice, inv: AudioInventory): List<SourceOption> {
    val apps = inv.sessions.filter { !it.systemSounds && it.pid > 0 }.distinctBy { it.pid }.map { s ->
        SourceOption(
            device = dev,
            source = RouteSource.Session(s.pid, s.appName),
            title = s.displayName ?: s.appName,
            subtitle = if (s.active) "Playing now" else "Idle",
            enabled = s.processCapture,
            disabledReason = s.processCaptureReason ?: "This app cannot be captured on its own",
            canSilenceLocal = s.canSilenceLocal,
            silenceReason = if (s.canSilenceLocal) null else "Silencing one app is not supported here; local playback continues.",
        )
    }
    val output = SourceOption(
        device = dev,
        source = RouteSource.Output,
        title = "Everything playing",
        subtitle = "The whole default output; it also keeps playing locally",
        enabled = inv.endpointCapture,
        disabledReason = "No output device to capture",
        canSilenceLocal = false,
        silenceReason = "Capturing the whole output includes local playback, so it cannot be silenced.",
    )
    val tone = SourceOption(dev, RouteSource.TestTone, "Test tone", "Quiet 440 Hz tone to check a route", true, null, false, "Nothing to silence for a test tone.")
    return apps + output + tone
}

@Composable
private fun RouteCard(r: ManagedRoute, onStop: () -> Unit) {
    Panel(Modifier.padding(bottom = 10.dp)) {
        Text("${r.spec.source.label()}: ${r.sourceName} → ${r.destName}", style = MaterialTheme.typography.titleMedium)
        r.sink?.endpointName?.let { Text("on $it", style = MaterialTheme.typography.bodySmall, color = Palette.Muted) }
        val stateText = when (val p = r.phase) {
            RoutePhase.PreparingDestination -> "Preparing destination"
            RoutePhase.Starting -> r.info?.state?.let(::routeStateText) ?: "Starting stream, verifying playback"
            RoutePhase.Active -> "Active" + if (r.info?.localSilenced == true) " · silenced on ${r.sourceName}" else ""
            RoutePhase.Stopping -> "Stopping and restoring"
            is RoutePhase.Ended -> listOfNotNull("Stopped", outcomeText(p.outcome).ifEmpty { null }).joinToString(" · ")
        }
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 6.dp)) {
            if (r.phase == RoutePhase.PreparingDestination || r.phase == RoutePhase.Starting || r.phase == RoutePhase.Stopping) {
                CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp, color = Palette.Cyan)
                Spacer(Modifier.width(8.dp))
            }
            Text(stateText, color = if (r.phase == RoutePhase.Active) Palette.Ok else Palette.Muted, style = MaterialTheme.typography.bodyMedium)
        }
        val src = r.info?.stats
        val dst = r.sink?.stats
        if (r.phase == RoutePhase.Active && src != null) {
            Text(
                listOfNotNull(
                    src.latencyMs?.let { "~${it.toInt()} ms" },
                    "%.0f kb/s".format(src.bitrateKbps),
                    dst?.let { "lost ${it.lost}" },
                    dst?.let { "jitter %.1f ms".format(it.jitterMs) },
                ).joinToString(" · "),
                fontFamily = Mono, style = MaterialTheme.typography.bodySmall, color = Palette.Muted, modifier = Modifier.padding(top = 4.dp),
            )
            LinearProgressIndicator(
                progress = { (dst?.peak ?: src.peak).coerceIn(0f, 1f) },
                modifier = Modifier.fillMaxWidth().padding(top = 6.dp).semantics { contentDescription = "Output level" },
                color = Palette.Violet, trackColor = Palette.SurfaceHigh,
            )
        }
        val ended = r.phase as? RoutePhase.Ended
        (ended?.error ?: r.info?.error)?.let { Text(it, color = Palette.Warn, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 4.dp)) }
        (ended?.note ?: r.info?.note)?.let { Text(it, color = Palette.Muted, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 4.dp)) }
        if (r.live && r.phase != RoutePhase.Stopping) {
            OutlinedButton(onClick = onStop, modifier = Modifier.padding(top = 8.dp)) { Text("Stop and restore") }
        }
    }
}
