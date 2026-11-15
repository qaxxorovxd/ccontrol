package dev.ccontrol.app.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material.icons.outlined.DesktopWindows
import androidx.compose.material.icons.outlined.Laptop
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.ccontrol.app.ui.theme.Palette
import dev.ccontrol.core.net.LinkState

@Composable
fun Panel(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Card(
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(16.dp),
        colors = CardDefaults.cardColors(containerColor = Palette.Surface),
        border = BorderStroke(1.dp, Palette.Outline),
    ) { Column(Modifier.padding(16.dp), content = content) }
}

@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text.uppercase(),
        style = MaterialTheme.typography.labelSmall,
        color = Palette.Muted,
        modifier = modifier.padding(top = 20.dp, bottom = 8.dp, start = 4.dp),
    )
}

data class StatusLook(val label: String, val color: Color)

fun LinkState?.look(): StatusLook = when (this) {
    is LinkState.Online -> StatusLook("Online", Palette.Ok)
    LinkState.Connecting -> StatusLook("Connecting", Palette.Cyan)
    is LinkState.Offline -> StatusLook("Offline", Palette.Muted)
    is LinkState.NotTrusted -> StatusLook("Permission required", Palette.Warn)
    is LinkState.IdentityChanged -> StatusLook("Identity changed", Palette.Bad)
    is LinkState.Incompatible -> StatusLook("Incompatible version", Palette.Bad)
    LinkState.Idle, null -> StatusLook("Paused", Palette.Muted)
}

@Composable
fun StatusPill(state: LinkState?) {
    val l = state.look()
    Row(
        Modifier
            .border(1.dp, l.color.copy(alpha = 0.35f), RoundedCornerShape(50))
            .padding(horizontal = 10.dp, vertical = 3.dp)
            .semantics { contentDescription = "Status: ${l.label}" },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(7.dp).background(l.color, CircleShape))
        Spacer(Modifier.width(6.dp))
        Text(l.label, style = MaterialTheme.typography.bodySmall, color = l.color)
    }
}

fun osIcon(os: String?): ImageVector = when (os) {
    "windows" -> Icons.Outlined.DesktopWindows
    "linux" -> Icons.Outlined.Laptop
    else -> Icons.Outlined.Computer
}

@Composable
fun OsBadge(os: String?, tint: Color = Palette.Violet) {
    Box(
        Modifier.size(44.dp).background(Palette.SurfaceHigh, RoundedCornerShape(12.dp)),
        contentAlignment = Alignment.Center,
    ) { Icon(osIcon(os), contentDescription = null, tint = tint) }
}

@Composable
fun EmptyState(title: String, body: String, modifier: Modifier = Modifier, action: @Composable () -> Unit = {}) {
    Column(modifier.fillMaxWidth().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(title, style = MaterialTheme.typography.titleMedium)
        Text(body, style = MaterialTheme.typography.bodyMedium, color = Palette.Muted)
        Spacer(Modifier.size(8.dp))
        action()
    }
}

fun formatBytes(b: Long): String {
    val gb = b / 1_073_741_824.0
    return if (gb >= 1) "%.1f GB".format(gb) else "%.0f MB".format(b / 1_048_576.0)
}

fun formatUptime(s: Long): String {
    val d = s / 86400; val h = (s % 86400) / 3600; val m = (s % 3600) / 60
    return when { d > 0 -> "${d}d ${h}h"; h > 0 -> "${h}h ${m}m"; else -> "${m}m" }
}
