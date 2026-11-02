package dev.phonebridge.app.ui.components

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/**
 * Phone and PC joined by the link line: the design's signature element. While [live] the dashes
 * flow from phone to PC in the accent colour; paired but idle they rest, dimmed; unpaired they fade further.
 */
@Composable
fun LinkIndicator(phone: String, pc: String?, live: Boolean) {
    val badge = MaterialTheme.colorScheme.primaryContainer
    val onBadge = MaterialTheme.colorScheme.onPrimaryContainer
    val outline = MaterialTheme.colorScheme.outline
    val line by animateColorAsState(
        when {
            live -> MaterialTheme.colorScheme.primary
            pc != null -> outline
            else -> outline.copy(alpha = 0.35f)
        },
        label = "link colour",
    )
    // One dash period (dash + gap) per 900 ms, only while live: an idle Home draws no frames.
    // Read in the draw phase only, so the flow redraws the line without recomposing.
    val phase = if (live) {
        rememberInfiniteTransition(label = "link flow")
            .animateFloat(0f, 1f, infiniteRepeatable(tween(900, easing = LinearEasing), RepeatMode.Restart), label = "link phase")
    } else {
        null
    }
    Row(verticalAlignment = Alignment.Top, modifier = Modifier.fillMaxWidth()) {
        DeviceBadge(phone, badge, onBadge, isPhone = true)
        Box(Modifier.weight(1f).height(44.dp), contentAlignment = Alignment.Center) {
            Canvas(Modifier.fillMaxWidth().height(4.dp).padding(horizontal = 8.dp)) {
                val dash = 4.dp.toPx()
                val gap = 8.dp.toPx()
                drawLine(
                    line,
                    Offset(0f, size.height / 2), Offset(size.width, size.height / 2),
                    strokeWidth = 3.dp.toPx(),
                    cap = StrokeCap.Round,
                    pathEffect = PathEffect.dashPathEffect(floatArrayOf(dash, gap), -(dash + gap) * (phase?.value ?: 0f)),
                )
            }
        }
        DeviceBadge(pc ?: "Not paired", badge, onBadge, isPhone = false)
    }
}

@Composable
private fun DeviceBadge(name: String, bg: Color, fg: Color, isPhone: Boolean) {
    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Box(Modifier.size(44.dp).clip(RoundedCornerShape(14.dp)).background(bg), contentAlignment = Alignment.Center) {
            Canvas(Modifier.size(if (isPhone) 14.dp else 20.dp, if (isPhone) 22.dp else 14.dp)) {
                drawRoundRect(fg, cornerRadius = CornerRadius(6f, 6f), style = Stroke(width = 4f))
            }
        }
        Text(name, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.width(110.dp), textAlign = TextAlign.Center)
    }
}
