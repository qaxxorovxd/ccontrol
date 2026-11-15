package dev.ccontrol.app.ui.screens

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.ccontrol.app.CControlApp
import dev.ccontrol.app.ui.components.EmptyState
import dev.ccontrol.app.ui.components.Panel
import dev.ccontrol.app.ui.theme.Palette
import dev.ccontrol.core.net.LinkState
import dev.ccontrol.core.net.RemoteException
import dev.ccontrol.core.protocol.Action
import dev.ccontrol.core.protocol.ActionResult
import dev.ccontrol.core.protocol.AppWindow
import dev.ccontrol.core.protocol.Scopes
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

@Composable
fun AppsScreen(app: CControlApp, deviceId: String, onBack: () -> Unit) {
    val states by app.connections.states.collectAsStateWithLifecycle()
    val online = states[deviceId] as? LinkState.Online
    val canClose = online?.scopes?.contains(Scopes.PROCESS_TERMINATE) == true
    val scope = rememberCoroutineScope()
    var windows by remember { mutableStateOf<List<AppWindow>?>(null) }
    var unsupported by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var confirm by remember { mutableStateOf<AppWindow?>(null) }

    suspend fun refresh() {
        try {
            windows = (app.connections.call(deviceId, Action.AppList) as ActionResult.Apps).windows
            error = null
        } catch (e: RemoteException) {
            if (e.error.code == "unsupported") unsupported = e.error.message else error = e.userMessage()
        } catch (t: Throwable) {
            error = t.userMessage()
        }
    }

    LaunchedEffect(online != null) {
        while (isActive && online != null && unsupported == null) {
            refresh()
            delay(3000)
        }
    }

    Column(Modifier.fillMaxSize().safeDrawingPadding().padding(horizontal = 16.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 8.dp)) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
            Text("Apps & windows", style = MaterialTheme.typography.headlineSmall)
        }
        error?.let { Text(it, color = Palette.Warn, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(4.dp)) }
        when {
            unsupported != null -> EmptyState("Not available on this computer", unsupported!!)
            online == null -> EmptyState("Computer is offline", "Windows appear when it reconnects.")
            windows == null -> LinearProgressIndicator(Modifier.fillMaxWidth().padding(top = 16.dp), color = Palette.Violet)
            windows!!.isEmpty() -> EmptyState("No open windows", "Nothing is open on the computer's desktop.")
            else -> LazyColumn(contentPadding = PaddingValues(vertical = 8.dp)) {
                items(windows!!, key = { it.windowId }) { w ->
                    Panel(Modifier.padding(bottom = 8.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Column(Modifier.weight(1f)) {
                                Text(w.title, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
                                Text(
                                    listOfNotNull(w.processName, "in front".takeIf { w.focused }, "minimized".takeIf { w.minimized }).joinToString(" · "),
                                    style = MaterialTheme.typography.bodySmall, color = Palette.Muted,
                                )
                            }
                            if (canClose) TextButton(onClick = { confirm = w }) { Text("Close") }
                        }
                    }
                }
                if (!canClose) item {
                    Text("Closing windows needs the process.terminate permission, granted on the computer.", style = MaterialTheme.typography.bodySmall, color = Palette.Muted)
                }
            }
        }
    }

    confirm?.let { w ->
        AlertDialog(
            onDismissRequest = { confirm = null },
            title = { Text("Close window?") },
            text = { Text("“${w.title}” will be asked to close, like clicking its X. If it has unsaved changes it will ask on the computer.") },
            confirmButton = {
                TextButton(onClick = {
                    confirm = null
                    scope.launch {
                        try {
                            app.connections.call(deviceId, Action.AppClose(w.windowId, w.pid))
                            delay(600)
                        } catch (t: Throwable) {
                            error = t.userMessage()
                        }
                        refresh()
                    }
                }) { Text("Close") }
            },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text("Cancel") } },
        )
    }
}
