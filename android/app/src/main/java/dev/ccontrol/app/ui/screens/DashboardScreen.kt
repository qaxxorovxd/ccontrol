package dev.ccontrol.app.ui.screens

import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.PushPin
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.ccontrol.app.CControlApp
import dev.ccontrol.app.ui.components.EmptyState
import dev.ccontrol.app.ui.components.OsBadge
import dev.ccontrol.app.ui.components.Panel
import dev.ccontrol.app.ui.components.SectionLabel
import dev.ccontrol.app.ui.components.StatusPill
import dev.ccontrol.app.ui.theme.Palette
import dev.ccontrol.core.net.LinkState

const val LOCAL_NETWORK_PERMISSION = "android.permission.ACCESS_LOCAL_NETWORK"

/** Android 17 (API 37) gates LAN access behind a runtime permission. */
fun needsLocalNetworkPermission() = Build.VERSION.SDK_INT >= 37

@Composable
fun LocalNetworkGate(onGranted: () -> Unit = {}) {
    if (!needsLocalNetworkPermission()) return
    val ctx = LocalContext.current
    var granted by remember {
        mutableStateOf(ContextCompat.checkSelfPermission(ctx, LOCAL_NETWORK_PERMISSION) == PackageManager.PERMISSION_GRANTED)
    }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        granted = it
        if (it) onGranted()
    }
    LaunchedEffect(Unit) { if (!granted) launcher.launch(LOCAL_NETWORK_PERMISSION) }
    if (!granted) {
        Panel(Modifier.padding(bottom = 12.dp)) {
            Text("Local network access is off", style = MaterialTheme.typography.titleMedium, color = Palette.Warn)
            Text(
                "ccontrol needs it to find and reach computers on your Wi-Fi. Nothing is sent outside your network.",
                style = MaterialTheme.typography.bodyMedium, color = Palette.Muted,
            )
            TextButton(onClick = { launcher.launch(LOCAL_NETWORK_PERMISSION) }) { Text("Allow") }
        }
    }
}

@Composable
fun DashboardScreen(app: CControlApp, onOpen: (String) -> Unit, onAdd: () -> Unit) {
    val devices by app.repo.devices.collectAsStateWithLifecycle()
    val states by app.connections.states.collectAsStateWithLifecycle()

    Scaffold(
        containerColor = Palette.Background,
        floatingActionButton = {
            ExtendedFloatingActionButton(
                onClick = onAdd,
                icon = { Icon(Icons.Filled.Add, contentDescription = null) },
                text = { Text("Add computer") },
                containerColor = Palette.Violet,
                contentColor = Palette.Background,
            )
        },
    ) { pad ->
        LazyColumn(
            Modifier.fillMaxSize().padding(pad).safeDrawingPadding(),
            contentPadding = PaddingValues(16.dp, 8.dp, 16.dp, 96.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            item {
                Column(Modifier.padding(top = 8.dp, bottom = 8.dp)) {
                    Text("ccontrol", style = MaterialTheme.typography.headlineSmall)
                    val online = devices.count { states[it.deviceId] is LinkState.Online }
                    Text(
                        if (devices.isEmpty()) "No computers yet" else "$online of ${devices.size} online",
                        style = MaterialTheme.typography.bodyMedium, color = Palette.Muted,
                    )
                }
            }
            item { LocalNetworkGate(onGranted = app.connections::reconnectNow) }
            if (devices.isEmpty()) {
                item {
                    EmptyState(
                        "Pair your first computer",
                        "Install the ccontrol agent on a Windows or Linux PC, open its settings page, then add it here.",
                    ) { TextButton(onClick = onAdd) { Text("Add computer") } }
                }
            } else {
                item { SectionLabel("Computers") }
                items(devices, key = { it.deviceId }) { d ->
                    val st = states[d.deviceId]
                    Panel(Modifier.clickable { onOpen(d.deviceId) }) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            OsBadge(d.osFamily, tint = if (st is LinkState.Online) Palette.Violet else Palette.Muted)
                            Spacer(Modifier.width(14.dp))
                            Column(Modifier.weight(1f)) {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Text(d.displayName, style = MaterialTheme.typography.titleMedium)
                                    if (d.pinned) Icon(Icons.Filled.PushPin, "Pinned", Modifier.padding(start = 6.dp).width(14.dp), tint = Palette.Muted)
                                }
                                Text(
                                    listOfNotNull(d.manifest?.osVersion ?: d.osFamily, d.lastHost).joinToString(" · "),
                                    style = MaterialTheme.typography.bodySmall, color = Palette.Muted,
                                )
                            }
                            StatusPill(st)
                        }
                        if (st is LinkState.NotTrusted || st is LinkState.IdentityChanged || st is LinkState.Incompatible) {
                            Text(
                                when (st) {
                                    is LinkState.NotTrusted -> st.reason
                                    is LinkState.IdentityChanged -> st.reason
                                    is LinkState.Incompatible -> st.reason
                                    else -> ""
                                },
                                style = MaterialTheme.typography.bodySmall, color = Palette.Warn,
                                modifier = Modifier.fillMaxWidth().padding(top = 10.dp),
                            )
                        }
                    }
                }
            }
        }
    }
}
