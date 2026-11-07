package dev.phonebridge.app.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import dev.chrisbanes.haze.HazeState
import dev.chrisbanes.haze.hazeSource
import androidx.compose.ui.platform.LocalContext
import android.os.Build
import dev.phonebridge.app.Prefs
import dev.phonebridge.app.ui.components.LocalGlass
import dev.phonebridge.app.ui.components.LocalHazeState
import dev.phonebridge.app.ui.components.LocalModes
import dev.phonebridge.app.ui.rememberSlowFrames
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.text.TextStyle

/** Amber for codes: the shared `attention` token. */
val Amber = Tokens.Light.attention
val AmberDark = Tokens.Dark.attention

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

/**
 * App-wide theme choice, observable so a change applies at once: the mode (Prefs.theme: 0 system, 1 light,
 * 2 dark), the glass look (Prefs.themeLook), reduced transparency and the manual lite switch.
 */
object ThemeChoice {
    val mode = mutableIntStateOf(0)
    val look = mutableIntStateOf(0)
    val reduceTransparency = mutableStateOf(false)
    val liteForced = mutableStateOf(false)
}

/** Copies the saved appearance into the observable choice (call where the app starts or resumes). */
fun ThemeChoice.loadFrom(p: Prefs) {
    mode.intValue = p.theme
    look.intValue = p.themeLook
    reduceTransparency.value = p.reduceTransparency
    liteForced.value = p.liteForced
}

@Composable
fun isDark(choice: ThemeChoice = ThemeChoice): Boolean = when (choice.mode.intValue) {
    1 -> false
    2 -> true
    else -> isSystemInDarkTheme()
}

/**
 * The app's theme: the colour scheme for the chosen look (Native glass takes the system's dynamic colour from
 * Android 12), and the glass material and modes the components read through `LocalGlass` and `LocalModes`.
 */
@Composable
fun RelayTheme(choice: ThemeChoice = ThemeChoice, content: @Composable () -> Unit) {
    val dark = isDark(choice)
    val look = if (choice.look.intValue == 1) Look.Relay else Look.Native
    val context = LocalContext.current
    val scheme = colorSchemeFor(look, dark, Build.VERSION.SDK_INT) { d ->
        if (Build.VERSION.SDK_INT < 31) null
        else if (d) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
    }
    // Blur is in use while glass is Relay glass and the phone is not already lite; slow frames then turn lite on.
    val blurActive = look == Look.Relay && !choice.liteForced.value && blurMode(Build.VERSION.SDK_INT, Modes()) == BlurMode.Haze
    val signals = rememberLiteSignals(forced = choice.liteForced.value, slowFrames = rememberSlowFrames(blurActive))
    val modes = Modes(
        reducedTransparency = choice.reduceTransparency.value,
        lite = liteReason(signals) != null,
    )
    val glass = resolve(materialFor(look, dark), modes)
    val haze = remember { HazeState() }
    CompositionLocalProvider(LocalGlass provides glass, LocalModes provides modes, LocalHazeState provides haze) {
        MaterialTheme(
            colorScheme = scheme,
            typography = RelayTypography,
            shapes = RelayShapes,
        ) {
            // The whole app is the backdrop that glass blurs.
            Box(Modifier.fillMaxSize().hazeSource(haze)) { content() }
        }
    }
}
