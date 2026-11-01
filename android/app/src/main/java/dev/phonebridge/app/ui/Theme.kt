package dev.phonebridge.app.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle

/** Amber for codes: the shared `accent` token. */
val Amber = Tokens.Light.accent
val AmberDark = Tokens.Dark.accent

/*
 * Colours come from the shared tokens (design/tokens.toml): surface, surface_variant ->
 * surfaceContainer, outline, text -> onSurface, text_muted -> onSurfaceVariant, primary, on_primary.
 * The tokens have no container roles, so those keep the hand-picked cobalt tints from design v2.
 */
fun relayLightScheme(): ColorScheme = lightColorScheme(
    primary = Tokens.Light.primary, onPrimary = Tokens.Light.onPrimary,
    primaryContainer = Color(0xFFDDE1FF), onPrimaryContainer = Color(0xFF001257),
    secondaryContainer = Color(0xFFE0E2F2), onSecondaryContainer = Color(0xFF191B28),
    surface = Tokens.Light.surface, surfaceContainer = Tokens.Light.surfaceVariant,
    onSurface = Tokens.Light.text, onSurfaceVariant = Tokens.Light.textMuted, outline = Tokens.Light.outline,
    error = Tokens.Light.danger,
)

fun relayDarkScheme(): ColorScheme = darkColorScheme(
    primary = Tokens.Dark.primary, onPrimary = Tokens.Dark.onPrimary,
    primaryContainer = Color(0xFF233CA9), onPrimaryContainer = Color(0xFFDDE1FF),
    secondaryContainer = Color(0xFF42455A), onSecondaryContainer = Color(0xFFE0E2F2),
    surface = Tokens.Dark.surface, surfaceContainer = Tokens.Dark.surfaceVariant,
    onSurface = Tokens.Dark.text, onSurfaceVariant = Tokens.Dark.textMuted, outline = Tokens.Dark.outline,
    error = Tokens.Dark.danger,
)

/*
 * Type scale mapping: caption -> labelSmall + bodySmall, body -> bodyMedium, subtitle -> titleMedium,
 * title -> titleLarge, display -> headlineMedium. Other roles keep the M3 defaults.
 */
val RelayTypography: Typography = Typography().let { d ->
    fun TextStyle.sized(s: androidx.compose.ui.unit.TextUnit) = copy(fontSize = s)
    d.copy(
        labelSmall = d.labelSmall.sized(Tokens.Type.Caption),
        bodySmall = d.bodySmall.sized(Tokens.Type.Caption),
        bodyMedium = d.bodyMedium.sized(Tokens.Type.Body),
        titleMedium = d.titleMedium.sized(Tokens.Type.Subtitle),
        titleLarge = d.titleLarge.sized(Tokens.Type.Title),
        headlineMedium = d.headlineMedium.sized(Tokens.Type.Display),
    )
}

val RelayShapes = Shapes(
    extraSmall = RoundedCornerShape(Tokens.Radius[0]),
    small = RoundedCornerShape(Tokens.Radius[1]),
    medium = RoundedCornerShape(Tokens.Radius[2]),
    large = RoundedCornerShape(Tokens.Radius[3]),
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
fun RelayTheme(choice: ThemeChoice = ThemeChoice, content: @Composable () -> Unit) {
    val dark = when (choice.mode.intValue) {
        1 -> false
        2 -> true
        else -> isSystemInDarkTheme()
    }
    MaterialTheme(
        colorScheme = if (dark) relayDarkScheme() else relayLightScheme(),
        typography = RelayTypography,
        shapes = RelayShapes,
        content = content,
    )
}
