package dev.phonebridge.app.ui.components

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
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.unit.dp
import dev.phonebridge.app.ui.GlassMaterial
import dev.phonebridge.app.ui.Modes
import dev.phonebridge.app.ui.Tokens

/** The modes in effect for this part of the screen (set by `RelayTheme`). */
val LocalModes = staticCompositionLocalOf { Modes() }

/** The resolved glass material in effect (set by `RelayTheme`): opacity is 1 wherever the modes ask for solid. */
val LocalGlass = staticCompositionLocalOf { Tokens.Materials[0] }

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
    Box(
        modifier
            .clip(shape)
            .background(glass.tint.copy(alpha = glass.opacity))
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
