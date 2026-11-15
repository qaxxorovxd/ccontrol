package dev.ccontrol.app.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.VerifiedUser
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.ccontrol.app.CControlApp
import dev.ccontrol.app.ui.components.Panel
import dev.ccontrol.app.ui.theme.Mono
import dev.ccontrol.app.ui.theme.Palette
import dev.ccontrol.core.data.PairedDevice
import dev.ccontrol.core.net.AgentSession
import dev.ccontrol.core.net.ConnectException
import dev.ccontrol.core.net.PairingFlow
import dev.ccontrol.core.net.PairingUpdate
import dev.ccontrol.core.protocol.HelloAck
import dev.ccontrol.core.protocol.Scopes
import dev.ccontrol.core.security.Fingerprint
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

sealed interface PairUi {
    data object Connecting : PairUi
    data class Awaiting(val sas: String, val computer: String, val verifiedByQr: Boolean, val serverShortId: String) : PairUi
    data class Done(val deviceId: String, val name: String, val scopes: List<String>, val alreadyPaired: Boolean) : PairUi
    data class Failed(val message: String) : PairUi
}

class PairingViewModel(private val app: CControlApp, private val target: PairingTarget) : ViewModel() {
    private val _ui = MutableStateFlow<PairUi>(PairUi.Connecting)
    val ui: StateFlow<PairUi> = _ui
    private var session: AgentSession? = null
    private var job: Job? = null

    init { start() }

    fun start() {
        job?.cancel()
        _ui.value = PairUi.Connecting
        job = viewModelScope.launch {
            try {
                val identity = withContext(Dispatchers.Default) { app.identity() }
                // With a QR code, the expected fingerprint is pinned from the start:
                // a different computer at that address fails the TLS handshake.
                val s = AgentSession.open(target.host, target.port, identity, target.fingerprint, app.appInfo)
                session = s
                if (s.helloAck.paired) {
                    finish(s, s.helloAck, s.helloAck.grantedScopes, already = true)
                    return@launch
                }
                val result = PairingFlow.run(s, identity, Scopes.DEFAULT_REQUEST) { u ->
                    if (u is PairingUpdate.AwaitingApproval) {
                        _ui.value = PairUi.Awaiting(
                            sas = u.sas,
                            computer = s.helloAck.deviceName,
                            verifiedByQr = target.fingerprint != null,
                            serverShortId = Fingerprint.short(s.serverFingerprint),
                        )
                    }
                }
                when (result) {
                    is PairingUpdate.Accepted -> finish(s, result.ack, result.grantedScopes, already = false)
                    is PairingUpdate.Rejected -> { s.close(); _ui.value = PairUi.Failed(result.reason) }
                    is PairingUpdate.AwaitingApproval -> {}
                }
            } catch (e: ConnectException) {
                _ui.value = PairUi.Failed(
                    when (e.kind) {
                        ConnectException.Kind.IdentityChanged -> "The computer at ${target.host} is not the one in the QR code. Pairing stopped for your safety."
                        ConnectException.Kind.Unreachable -> "Can't reach ${target.host}:${target.port}. Check that the agent is running, both devices are on the same network, and the firewall allows port ${target.port}."
                        else -> e.message ?: "Connection failed"
                    },
                )
            } catch (e: Exception) {
                _ui.value = PairUi.Failed(e.message ?: e.javaClass.simpleName)
            }
        }
    }

    private fun finish(s: AgentSession, ack: HelloAck, scopes: List<String>, already: Boolean) {
        val device = PairedDevice(
            deviceId = s.serverFingerprint,
            name = ack.deviceName,
            lastHost = target.host,
            port = target.port,
            osFamily = ack.manifest?.osFamily,
            grantedScopes = scopes,
            manifest = ack.manifest,
            pairedAt = app.repo.get(s.serverFingerprint)?.pairedAt ?: System.currentTimeMillis(),
            lastSeen = System.currentTimeMillis(),
        )
        app.connections.adopt(device, s)
        session = null
        _ui.value = PairUi.Done(device.deviceId, device.name, scopes, already)
    }

    override fun onCleared() {
        job?.cancel()
        session?.close()
    }
}

@Composable
fun PairingScreen(app: CControlApp, target: PairingTarget, onDone: (String?) -> Unit, onCancel: () -> Unit) {
    val vm: PairingViewModel = viewModel(factory = object : androidx.lifecycle.ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T = PairingViewModel(app, target) as T
    })
    val ui by vm.ui.collectAsStateWithLifecycle()

    Column(
        Modifier.fillMaxSize().safeDrawingPadding().padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        when (val s = ui) {
            PairUi.Connecting -> {
                CircularProgressIndicator(color = Palette.Violet)
                Spacer(Modifier.height(16.dp))
                Text("Connecting to ${target.name ?: target.host}…", color = Palette.Muted)
                Spacer(Modifier.height(24.dp))
                OutlinedButton(onClick = onCancel) { Text("Cancel") }
            }
            is PairUi.Awaiting -> {
                Text("Pair with ${s.computer}", style = MaterialTheme.typography.headlineSmall)
                Spacer(Modifier.height(8.dp))
                Text(
                    "Check that the computer shows the same code, then approve it there.",
                    color = Palette.Muted, style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(24.dp))
                Text(
                    s.sas.chunked(3).joinToString(" "),
                    fontFamily = Mono, fontSize = 44.sp, fontWeight = FontWeight.SemiBold, color = Palette.Cyan,
                    modifier = Modifier.semantics {
                        contentDescription = "Verification code ${s.sas.toList().joinToString(" ")}"
                        liveRegion = LiveRegionMode.Polite
                    },
                )
                Spacer(Modifier.height(24.dp))
                Panel {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        if (s.verifiedByQr) Icon(Icons.Filled.VerifiedUser, null, tint = Palette.Ok, modifier = Modifier.padding(end = 8.dp))
                        Text(
                            if (s.verifiedByQr) "Computer identity matches the QR code (${s.serverShortId})." else "Computer ID ${s.serverShortId}. Compare the code to be sure no one is intercepting.",
                            style = MaterialTheme.typography.bodySmall, color = Palette.Muted,
                        )
                    }
                }
                Spacer(Modifier.height(16.dp))
                CircularProgressIndicator(Modifier.height(20.dp), strokeWidth = 2.dp, color = Palette.Muted)
                Text("Waiting for approval…", color = Palette.Muted, style = MaterialTheme.typography.bodySmall)
                Spacer(Modifier.height(16.dp))
                OutlinedButton(onClick = onCancel) { Text("Codes don't match — cancel") }
            }
            is PairUi.Done -> {
                Icon(Icons.Filled.VerifiedUser, null, tint = Palette.Ok, modifier = Modifier.height(48.dp))
                Spacer(Modifier.height(12.dp))
                Text(if (s.alreadyPaired) "Already paired with ${s.name}" else "Paired with ${s.name}", style = MaterialTheme.typography.headlineSmall)
                Spacer(Modifier.height(8.dp))
                Text(
                    if (s.scopes.isEmpty()) "No permissions were granted. You can change this on the computer." else "Granted: ${s.scopes.joinToString(", ")}",
                    color = Palette.Muted, style = MaterialTheme.typography.bodySmall,
                )
                Spacer(Modifier.height(24.dp))
                Button(
                    onClick = { onDone(s.deviceId) },
                    colors = ButtonDefaults.buttonColors(containerColor = Palette.Violet, contentColor = Palette.Background),
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Open ${s.name}") }
            }
            is PairUi.Failed -> {
                Text("Pairing didn't complete", style = MaterialTheme.typography.headlineSmall)
                Spacer(Modifier.height(8.dp))
                Text(s.message, color = Palette.Muted, style = MaterialTheme.typography.bodyMedium)
                Spacer(Modifier.height(24.dp))
                Button(onClick = vm::start, colors = ButtonDefaults.buttonColors(containerColor = Palette.Violet, contentColor = Palette.Background)) { Text("Try again") }
                OutlinedButton(onClick = onCancel, modifier = Modifier.padding(top = 8.dp)) { Text("Back") }
            }
        }
    }
}
