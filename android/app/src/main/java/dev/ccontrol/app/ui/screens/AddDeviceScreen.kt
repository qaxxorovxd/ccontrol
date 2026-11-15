package dev.ccontrol.app.ui.screens

import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import dev.ccontrol.app.CControlApp
import dev.ccontrol.app.ui.components.OsBadge
import dev.ccontrol.app.ui.components.Panel
import dev.ccontrol.app.ui.components.SectionLabel
import dev.ccontrol.app.ui.theme.Palette
import dev.ccontrol.core.net.PairingUri
import dev.ccontrol.core.protocol.ProtocolInfo

data class PairingTarget(val host: String, val port: Int, val fingerprint: String?, val name: String?)

@Composable
fun AddDeviceScreen(app: CControlApp, onBack: () -> Unit, onPair: (PairingTarget) -> Unit) {
    val ctx = LocalContext.current
    val discovered by app.connections.discovered.collectAsStateWithLifecycle()
    val paired by app.repo.devices.collectAsStateWithLifecycle()
    var host by rememberSaveable { mutableStateOf("") }
    var port by rememberSaveable { mutableStateOf(ProtocolInfo.DEFAULT_PORT.toString()) }
    var scanError by rememberSaveable { mutableStateOf<String?>(null) }

    fun scan() {
        scanError = null
        val opts = GmsBarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build()
        GmsBarcodeScanning.getClient(ctx, opts).startScan()
            .addOnSuccessListener { code ->
                val uri = code.rawValue?.let(PairingUri::parse)
                if (uri == null) scanError = "That QR code is not a ccontrol pairing code."
                else onPair(PairingTarget(uri.host, uri.port, uri.fingerprint, uri.name))
            }
            .addOnFailureListener { scanError = "Scanner unavailable (${it.message}). Enter the address instead." }
    }

    Column(
        Modifier.fillMaxSize().safeDrawingPadding().verticalScroll(rememberScrollState()).padding(16.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
            Text("Add computer", style = MaterialTheme.typography.headlineSmall)
        }
        LocalNetworkGate(onGranted = app.connections::reconnectNow)

        Button(
            onClick = ::scan,
            modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
            colors = ButtonDefaults.buttonColors(containerColor = Palette.Violet, contentColor = Palette.Background),
        ) {
            Icon(Icons.Filled.QrCodeScanner, null); Spacer(Modifier.width(8.dp)); Text("Scan pairing QR code")
        }
        Text(
            "On the computer run “ccontrol-agent ui” (or open its tray settings) to show the code. Scanning also proves you are talking to the right computer.",
            style = MaterialTheme.typography.bodySmall, color = Palette.Muted, modifier = Modifier.padding(top = 6.dp),
        )
        scanError?.let { Text(it, color = Palette.Warn, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 6.dp)) }

        SectionLabel("On this network")
        val pairedIds = paired.map { it.deviceId }.toSet()
        val list = discovered.values.sortedBy { it.name.lowercase() }
        if (list.isEmpty()) {
            Panel {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.width(18.dp), strokeWidth = 2.dp, color = Palette.Cyan)
                    Spacer(Modifier.width(12.dp))
                    Text(
                        "Searching… If nothing appears, your network may block multicast (mDNS). Use the QR code or the address below.",
                        style = MaterialTheme.typography.bodySmall, color = Palette.Muted,
                    )
                }
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            list.forEach { a ->
                val already = a.deviceId in pairedIds
                Panel(Modifier.clickable(enabled = !already) { onPair(PairingTarget(a.host, a.port, null, a.name)) }) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        OsBadge(a.osFamily)
                        Spacer(Modifier.width(12.dp))
                        Column(Modifier.weight(1f)) {
                            Text(a.name, style = MaterialTheme.typography.titleMedium)
                            Text("${a.host}:${a.port}", style = MaterialTheme.typography.bodySmall, color = Palette.Muted)
                        }
                        Text(if (already) "Paired" else "Pair", color = if (already) Palette.Muted else Palette.Violet)
                    }
                }
            }
        }

        SectionLabel("Enter address")
        Panel {
            OutlinedTextField(
                value = host, onValueChange = { host = it.trim() }, label = { Text("IP address or hostname") },
                singleLine = true, modifier = Modifier.fillMaxWidth(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
            )
            OutlinedTextField(
                value = port, onValueChange = { port = it.filter(Char::isDigit).take(5) }, label = { Text("Port") },
                singleLine = true, modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            )
            val p = port.toIntOrNull()
            OutlinedButton(
                onClick = { onPair(PairingTarget(host, p!!, null, null)) },
                enabled = host.isNotBlank() && p != null && p in 1..65535,
                modifier = Modifier.padding(top = 12.dp),
            ) { Text("Connect") }
        }
    }
}
