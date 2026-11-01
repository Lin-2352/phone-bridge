package dev.phonebridge.app.ui.components

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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** Phone and PC joined by the link line: the design's signature element. */
@Composable
fun LinkIndicator(phone: String, pc: String?, running: Boolean) {
    val badge = MaterialTheme.colorScheme.primaryContainer
    val onBadge = MaterialTheme.colorScheme.onPrimaryContainer
    val line = MaterialTheme.colorScheme.outline
    Row(verticalAlignment = Alignment.Top, modifier = Modifier.fillMaxWidth()) {
        DeviceBadge(phone, badge, onBadge, isPhone = true)
        Box(Modifier.weight(1f).height(44.dp), contentAlignment = Alignment.Center) {
            Canvas(Modifier.fillMaxWidth().height(2.dp).padding(horizontal = 8.dp)) {
                drawLine(
                    if (pc != null && running) line else line.copy(alpha = 0.35f),
                    Offset(0f, size.height / 2), Offset(size.width, size.height / 2),
                    strokeWidth = 4f, pathEffect = PathEffect.dashPathEffect(floatArrayOf(4f, 10f)),
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
