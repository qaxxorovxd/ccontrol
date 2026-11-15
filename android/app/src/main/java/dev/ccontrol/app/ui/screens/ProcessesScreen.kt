package dev.ccontrol.app.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.ccontrol.app.CControlApp
import dev.ccontrol.app.ui.components.EmptyState
import dev.ccontrol.app.ui.components.formatBytes
import dev.ccontrol.app.ui.theme.Mono
import dev.ccontrol.app.ui.theme.Palette
import dev.ccontrol.core.net.LinkState
import dev.ccontrol.core.protocol.Action
import dev.ccontrol.core.protocol.ActionResult
import dev.ccontrol.core.protocol.ProcessInfo
import dev.ccontrol.core.protocol.Scopes
import dev.ccontrol.core.protocol.TerminateOutcome
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

private enum class SortBy(val label: String) { Cpu("CPU"), Memory("Memory"), Name("Name") }

/** What the end-process dialog is showing. */
private sealed interface EndStep {
    data class Confirm(val p: ProcessInfo) : EndStep
    data class Working(val p: ProcessInfo, val force: Boolean) : EndStep
    /** Graceful stop did not finish; offer force quit. */
    data class OfferForce(val p: ProcessInfo, val why: String) : EndStep
    data class Result(val p: ProcessInfo, val outcome: TerminateOutcome) : EndStep
    data class Failed(val p: ProcessInfo, val message: String) : EndStep
}

@Composable
fun ProcessesScreen(app: CControlApp, deviceId: String, onBack: () -> Unit) {
    val states by app.connections.states.collectAsStateWithLifecycle()
    val online = states[deviceId] as? LinkState.Online
    val canKill = online?.scopes?.contains(Scopes.PROCESS_TERMINATE) == true
    val scope = rememberCoroutineScope()

    var list by remember { mutableStateOf<List<ProcessInfo>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var query by rememberSaveable { mutableStateOf("") }
    var sort by rememberSaveable { mutableStateOf(SortBy.Cpu) }
    var mineOnly by rememberSaveable { mutableStateOf(true) }
    var step by remember { mutableStateOf<EndStep?>(null) }

    suspend fun refresh() {
        try {
            list = (app.connections.call(deviceId, Action.ProcessList) as ActionResult.Processes).processes
            error = null
        } catch (t: Throwable) {
            error = t.userMessage()
        }
    }

    // Refresh every 3 s while visible and online (paused while a dialog is open).
    LaunchedEffect(online != null, step == null) {
        while (isActive && online != null && step == null) {
            refresh()
            delay(3000)
        }
    }

    fun end(p: ProcessInfo, force: Boolean) {
        step = EndStep.Working(p, force)
        scope.launch {
            step = try {
                val o = (app.connections.call(deviceId, Action.ProcessTerminate(p.pid, p.name, force)) as ActionResult.Terminated).outcome
                if (!o.exited && !force) EndStep.OfferForce(p, o.note ?: "It is still running.") else EndStep.Result(p, o)
            } catch (t: Throwable) {
                val msg = t.userMessage()
                if (!force && msg.startsWith("Not supported")) EndStep.OfferForce(p, msg.removePrefix("Not supported on this computer: "))
                else EndStep.Failed(p, msg)
            }
            refresh()
        }
    }

    Column(Modifier.fillMaxSize().safeDrawingPadding().padding(horizontal = 16.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 8.dp)) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
            Text("Processes", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
            list?.let { Text("${it.size}", color = Palette.Muted, fontFamily = Mono) }
        }
        Text(
            "Programs running on the computer. Prefer closing apps from “Apps & windows”; ending a process can lose unsaved work.",
            style = MaterialTheme.typography.bodySmall, color = Palette.Muted, modifier = Modifier.padding(horizontal = 4.dp),
        )
        OutlinedTextField(
            value = query, onValueChange = { query = it.take(64) }, singleLine = true,
            leadingIcon = { Icon(Icons.Filled.Search, null) }, placeholder = { Text("Search by name or PID") },
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )
        Row(Modifier.padding(vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SortBy.entries.forEach { s -> FilterChip(selected = sort == s, onClick = { sort = s }, label = { Text(s.label) }) }
            FilterChip(selected = mineOnly, onClick = { mineOnly = !mineOnly }, label = { Text("Mine") })
        }
        error?.let { Text(it, color = Palette.Warn, style = MaterialTheme.typography.bodySmall) }
        when {
            online == null -> EmptyState("Computer is offline", "Processes appear when it reconnects.")
            list == null && error == null -> LinearProgressIndicator(Modifier.fillMaxWidth().padding(top = 16.dp), color = Palette.Violet)
            else -> {
                val q = query.trim().lowercase()
                val shown = (list ?: emptyList())
                    .filter { !mineOnly || it.sameUser }
                    .filter { q.isEmpty() || it.name.lowercase().contains(q) || it.pid.toString() == q }
                    .let { l ->
                        when (sort) {
                            SortBy.Cpu -> l.sortedByDescending { it.cpuPercent }
                            SortBy.Memory -> l.sortedByDescending { it.memoryBytes }
                            SortBy.Name -> l.sortedBy { it.name.lowercase() }
                        }
                    }
                LazyColumn(contentPadding = PaddingValues(bottom = 24.dp)) {
                    items(shown, key = { it.pid }) { p ->
                        ProcessRow(p) { if (canKill && p.terminable) step = EndStep.Confirm(p) }
                    }
                }
            }
        }
    }

    step?.let { st ->
        val p = when (st) {
            is EndStep.Confirm -> st.p; is EndStep.Working -> st.p; is EndStep.OfferForce -> st.p
            is EndStep.Result -> st.p; is EndStep.Failed -> st.p
        }
        AlertDialog(
            onDismissRequest = { if (st !is EndStep.Working) step = null },
            title = { Text(p.name) },
            text = {
                Column {
                    Text("PID ${p.pid} · ${formatBytes(p.memoryBytes)}", fontFamily = Mono, style = MaterialTheme.typography.bodySmall, color = Palette.Muted)
                    p.exe?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Palette.Muted, maxLines = 2, overflow = TextOverflow.Ellipsis) }
                    Spacer(Modifier.size(12.dp))
                    when (st) {
                        is EndStep.Confirm -> Text("Ask ${p.name} to quit? It can save its work or ask you on the computer first.")
                        is EndStep.Working -> { Text(if (st.force) "Force quitting…" else "Asking it to quit…"); LinearProgressIndicator(Modifier.fillMaxWidth().padding(top = 8.dp), color = Palette.Violet) }
                        is EndStep.OfferForce -> Text("${st.why}\n\nForce quit ends it immediately. Unsaved work in ${p.name} will be lost.", color = Palette.Warn)
                        is EndStep.Result -> Text(
                            when {
                                st.outcome.exited && st.outcome.method == "force" -> "Force quit. The process has ended."
                                st.outcome.exited -> "It closed normally."
                                else -> st.outcome.note ?: "Still running."
                            },
                        )
                        is EndStep.Failed -> Text(st.message, color = Palette.Warn)
                    }
                }
            },
            confirmButton = {
                when (st) {
                    is EndStep.Confirm -> TextButton(onClick = { end(p, force = false) }) { Text("Quit") }
                    is EndStep.OfferForce -> TextButton(onClick = { end(p, force = true) }) { Text("Force quit", color = Palette.Bad) }
                    is EndStep.Working -> {}
                    else -> TextButton(onClick = { step = null }) { Text("OK") }
                }
            },
            dismissButton = {
                if (st is EndStep.Confirm || st is EndStep.OfferForce) TextButton(onClick = { step = null }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun ProcessRow(p: ProcessInfo, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(enabled = p.terminable, onClick = onClick).padding(vertical = 10.dp, horizontal = 4.dp)
            .semantics { contentDescription = "${p.name}, CPU ${p.cpuPercent.toInt()} percent, memory ${formatBytes(p.memoryBytes)}" + (p.protectedReason?.let { ", protected: $it" } ?: "") },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (!p.terminable) Icon(Icons.Filled.Lock, null, tint = Palette.Muted, modifier = Modifier.size(14.dp).padding(end = 4.dp))
                Text(p.name, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Text(
                "PID ${p.pid}" + (p.protectedReason?.let { " · $it" } ?: ""),
                style = MaterialTheme.typography.bodySmall, color = Palette.Muted, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
        }
        Column(horizontalAlignment = Alignment.End, modifier = Modifier.width(96.dp)) {
            Text("%.1f%%".format(p.cpuPercent), fontFamily = Mono, style = MaterialTheme.typography.bodySmall, color = if (p.cpuPercent > 20) Palette.Warn else Palette.Text)
            Text(formatBytes(p.memoryBytes), fontFamily = Mono, style = MaterialTheme.typography.bodySmall, color = Palette.Muted)
        }
    }
}
