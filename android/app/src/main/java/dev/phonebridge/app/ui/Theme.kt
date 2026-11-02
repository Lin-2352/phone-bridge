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
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.text.TextStyle

/** Amber for codes: the shared `accent` token. */
val Amber = Tokens.Light.accent
val AmberDark = Tokens.Dark.accent

/*
 * Colours come from the shared tokens (design/tokens.toml): surface -> background/surface,
 * surface_variant -> surfaceContainer, text -> onSurface, text_muted -> onSurfaceVariant,
 * primary, on_primary, danger -> error. The tokens have no container roles for primary/secondary,
 * so those keep the hand-picked cobalt tints from design v2.
 *
 * Container ladder (light and dark): every step is lighter than the page, like the PC where cards sit
 * above the page background: Lowest = page, Low, Container = the token surface_variant (cards),
 * High, Highest. Light fades up to white; dark fades up towards white by 12%. Strictly monotonic.
 */
class SurfaceLadder(val lowest: Color, val low: Color, val container: Color, val high: Color, val highest: Color)

fun surfaceLadder(surface: Color, variant: Color, top: Color): SurfaceLadder = SurfaceLadder(
    lowest = surface,
    low = lerp(surface, variant, 0.5f),
    container = variant,
    high = lerp(variant, top, 0.5f),
    highest = top,
)

/**
 * M3 `outline` is the strong border (>= 3:1 on every surface), derived from text_muted. The token
 * `outline` hairline (about 1.1:1) becomes `outlineVariant`.
 */
private fun strongOutline(surface: Color, textMuted: Color, amount: Float) = lerp(surface, textMuted, amount)

fun relayLightScheme(): ColorScheme {
    val t = Tokens.Light
    val l = surfaceLadder(t.surface, t.surfaceVariant, Color.White)
    return lightColorScheme(
        primary = t.primary, onPrimary = t.onPrimary,
        primaryContainer = Color(0xFFDDE1FF), onPrimaryContainer = Color(0xFF001257),
        secondaryContainer = Color(0xFFE0E2F2), onSecondaryContainer = Color(0xFF191B28),
        background = t.surface, onBackground = t.text,
        surface = t.surface, onSurface = t.text, onSurfaceVariant = t.textMuted,
        surfaceVariant = l.container, surfaceTint = t.primary,
        surfaceContainerLowest = l.lowest, surfaceContainerLow = l.low, surfaceContainer = l.container,
        surfaceContainerHigh = l.high, surfaceContainerHighest = l.highest,
        outline = strongOutline(t.surface, t.textMuted, 0.8f), outlineVariant = t.outline,
        inverseSurface = t.text, inverseOnSurface = t.surface, inversePrimary = Tokens.Dark.primary,
        error = t.danger, onError = Color.White,
        errorContainer = Color(0xFFFFDAD6), onErrorContainer = Color(0xFF410002),
    )
}

fun relayDarkScheme(): ColorScheme {
    val t = Tokens.Dark
    val l = surfaceLadder(t.surface, t.surfaceVariant, lerp(t.surfaceVariant, Color.White, 0.12f))
    return darkColorScheme(
        primary = t.primary, onPrimary = t.onPrimary,
        primaryContainer = Color(0xFF233CA9), onPrimaryContainer = Color(0xFFDDE1FF),
        secondaryContainer = Color(0xFF42455A), onSecondaryContainer = Color(0xFFE0E2F2),
        background = t.surface, onBackground = t.text,
        surface = t.surface, onSurface = t.text, onSurfaceVariant = t.textMuted,
        surfaceVariant = l.container, surfaceTint = t.primary,
        surfaceContainerLowest = l.lowest, surfaceContainerLow = l.low, surfaceContainer = l.container,
        surfaceContainerHigh = l.high, surfaceContainerHighest = l.highest,
        outline = strongOutline(t.surface, t.textMuted, 0.7f), outlineVariant = t.outline,
        inverseSurface = t.text, inverseOnSurface = t.surface, inversePrimary = Tokens.Light.primary,
        error = t.danger, onError = Color(0xFF690005),
        errorContainer = Color(0xFF93000A), onErrorContainer = Color(0xFFFFDAD6),
    )
}

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
fun isDark(choice: ThemeChoice = ThemeChoice): Boolean = when (choice.mode.intValue) {
    1 -> false
    2 -> true
    else -> isSystemInDarkTheme()
}

@Composable
fun RelayTheme(choice: ThemeChoice = ThemeChoice, content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = if (isDark(choice)) relayDarkScheme() else relayLightScheme(),
        typography = RelayTypography,
        shapes = RelayShapes,
        content = content,
    )
}
