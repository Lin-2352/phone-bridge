package dev.phonebridge.app.ui.components

import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.HazeState
import dev.chrisbanes.haze.HazeStyle
import dev.chrisbanes.haze.HazeTint
import dev.chrisbanes.haze.hazeEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.unit.dp
import dev.phonebridge.app.ui.GlassMaterial
import dev.phonebridge.app.ui.BlurMode
import dev.phonebridge.app.ui.Modes
import dev.phonebridge.app.ui.blurMode
import dev.phonebridge.app.ui.Tokens

/** The modes in effect for this part of the screen (set by `RelayTheme`). */
val LocalModes = staticCompositionLocalOf { Modes() }

/** The resolved glass material in effect (set by `RelayTheme`): opacity is 1 wherever the modes ask for solid. */
val LocalGlass = staticCompositionLocalOf { Tokens.Materials[0] }

/** The backdrop that glass blurs (set by `RelayTheme`, which draws the content as the blur source). Null where there is none. */
val LocalHazeState = staticCompositionLocalOf<HazeState?> { null }

/**
 * A panel drawn as glass: its tint at the resolved opacity and a hairline edge. Blur comes with M6.2 (Haze);
 * until then the glass is a tint over whatever is behind it. Text and status words never go directly on
 * glass: put them on a [SolidPanel] inside it.
 */
@Composable
fun GlassSurface(
    modifier: Modifier = Modifier,
    shape: Shape = RoundedCornerShape(Tokens.Shape.Card),
    content: @Composable BoxScope.() -> Unit,
) {
    val glass: GlassMaterial = LocalGlass.current
    val haze = LocalHazeState.current
    val mode = blurMode(Build.VERSION.SDK_INT, LocalModes.current)
    val tint = glass.tint.copy(alpha = glass.opacity)
    // Haze draws the blur and the tint itself; the scrim draws the tint; opaque is the solid surface.
    val blurred = if (mode == BlurMode.Haze && haze != null) {
        Modifier.hazeEffect(
            state = haze,
            style = HazeStyle(
                backgroundColor = MaterialTheme.colorScheme.surface,
                tint = HazeTint(tint),
                blurRadius = glass.blur.dp,
                noiseFactor = glass.grain,
            ),
        )
    } else {
        Modifier
    }
    val fill = when (mode) {
        BlurMode.Opaque -> Modifier.background(MaterialTheme.colorScheme.surfaceContainer)
        BlurMode.Scrim -> Modifier.background(tint)
        BlurMode.Haze -> Modifier
    }
    Box(
        modifier
            .clip(shape)
            .then(blurred)
            .then(fill)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = glass.borderAlpha), shape),
        content = content,
    )
}

/** An opaque panel for text, status words and controls: it sits inside glass and keeps the contrast. */
@Composable
fun SolidPanel(
    modifier: Modifier = Modifier,
    shape: Shape = RoundedCornerShape(Tokens.Shape.Card),
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier
            .clip(shape)
            .background(MaterialTheme.colorScheme.surfaceContainer),
        content = content,
    )
}
