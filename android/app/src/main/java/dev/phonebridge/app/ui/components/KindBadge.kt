package dev.phonebridge.app.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.phonebridge.app.ui.timeline.RecentItem

/** The letter badge in front of a Recent row: F(ile), C(lipboard) or L(ink). */
@Composable
fun KindBadge(kind: RecentItem.Kind) {
    val label = when (kind) {
        RecentItem.Kind.File -> "F"
        RecentItem.Kind.Clipboard -> "C"
        RecentItem.Kind.Link -> "L"
    }
    Box(Modifier.size(40.dp).clip(RoundedCornerShape(12.dp)).background(MaterialTheme.colorScheme.secondaryContainer), contentAlignment = Alignment.Center) {
        Text(label, color = MaterialTheme.colorScheme.onSecondaryContainer, fontWeight = FontWeight.Medium)
    }
}
