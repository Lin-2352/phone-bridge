package dev.phonebridge.app

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.ui.graphics.Color

/** Design v2 palette: Material 3 roles seeded from our cobalt, amber for codes. */
val Amber = Color(0xFFC9820F)
val AmberDark = Color(0xFFE8A93A)
private val Light = lightColorScheme(
    primary = Color(0xFF3651C5), onPrimary = Color.White,
    primaryContainer = Color(0xFFDDE1FF), onPrimaryContainer = Color(0xFF001257),
    secondaryContainer = Color(0xFFE0E2F2), onSecondaryContainer = Color(0xFF191B28),
    surface = Color(0xFFFBF8FF), surfaceContainer = Color(0xFFEFEDF6),
    onSurface = Color(0xFF1B1B21), onSurfaceVariant = Color(0xFF46464F), outline = Color(0xFF777680),
)
private val Dark = darkColorScheme(
    primary = Color(0xFFB9C3FF), onPrimary = Color(0xFF082587),
    primaryContainer = Color(0xFF233CA9), onPrimaryContainer = Color(0xFFDDE1FF),
    secondaryContainer = Color(0xFF42455A), onSecondaryContainer = Color(0xFFE0E2F2),
    surface = Color(0xFF121318), surfaceContainer = Color(0xFF1F1F25),
    onSurface = Color(0xFFE4E1E9), onSurfaceVariant = Color(0xFFC7C5D0), outline = Color(0xFF91909A),
)

/** App-wide theme choice (Prefs.theme: 0 system, 1 light, 2 dark), observable so a change applies at once. */
object ThemeChoice {
    val mode = mutableIntStateOf(0)
}

@Composable
fun isDark(): Boolean = when (ThemeChoice.mode.intValue) {
    1 -> false
    2 -> true
    else -> isSystemInDarkTheme()
}

@Composable
fun PbTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (isDark()) Dark else Light, content = content)
}
