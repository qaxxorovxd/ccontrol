package dev.ccontrol.app.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

object Palette {
    val Background = Color(0xFF0B0B0D)
    val Surface = Color(0xFF141418)
    val SurfaceHigh = Color(0xFF1C1C22)
    val Outline = Color(0xFF26262E)
    val Text = Color(0xFFE8E8EE)
    val Muted = Color(0xFF8A8A96)
    val Violet = Color(0xFF8B7CFF)
    val Cyan = Color(0xFF38D6E8)
    val Ok = Color(0xFF3ECF8E)
    val Warn = Color(0xFFF5A524)
    val Bad = Color(0xFFFF5D6C)
}

private val Scheme = darkColorScheme(
    primary = Palette.Violet,
    onPrimary = Color(0xFF0B0B0D),
    secondary = Palette.Cyan,
    onSecondary = Color(0xFF0B0B0D),
    background = Palette.Background,
    onBackground = Palette.Text,
    surface = Palette.Surface,
    onSurface = Palette.Text,
    surfaceVariant = Palette.SurfaceHigh,
    onSurfaceVariant = Palette.Muted,
    surfaceContainer = Palette.Surface,
    surfaceContainerHigh = Palette.SurfaceHigh,
    surfaceContainerHighest = Palette.SurfaceHigh,
    outline = Palette.Outline,
    outlineVariant = Palette.Outline,
    error = Palette.Bad,
)

private val Type = Typography(
    headlineSmall = TextStyle(fontSize = 22.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 0.sp),
    titleMedium = TextStyle(fontSize = 16.sp, fontWeight = FontWeight.SemiBold),
    bodyMedium = TextStyle(fontSize = 14.sp, lineHeight = 20.sp),
    bodySmall = TextStyle(fontSize = 12.sp, lineHeight = 16.sp),
    labelSmall = TextStyle(fontSize = 11.sp, fontWeight = FontWeight.Medium, letterSpacing = 0.8.sp),
)

val Mono = FontFamily.Monospace

@Composable
fun CControlTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = Scheme, typography = Type, content = content)
}
