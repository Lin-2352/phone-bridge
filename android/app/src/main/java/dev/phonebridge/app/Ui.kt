package dev.phonebridge.app

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** The one spacing scale. */
object Spacing {
    val xs: Dp = 4.dp
    val sm: Dp = 8.dp
    val md: Dp = 12.dp
    val lg: Dp = 16.dp
    val xl: Dp = 24.dp
}

/** The corner radii: small chips, cards, banners, big home card and dialogs. */
object Radius {
    val chip = 12.dp
    val card = 16.dp
    val banner = 20.dp
    val hero = 28.dp
}

/**
 * The app's dialog: one shape and type convention. The primary action is a
 * filled button on the right; the dismiss action is a TextButton on its left.
 * [destructive] draws the primary in the error colour (Remove).
 */
@Composable
fun RelayDialog(
    title: String?,
    onDismissRequest: () -> Unit,
    confirmText: String,
    onConfirm: () -> Unit,
    dismissText: String? = null,
    onDismiss: () -> Unit = onDismissRequest,
    destructive: Boolean = false,
    content: @Composable () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismissRequest,
        shape = RoundedCornerShape(Radius.hero),
        title = title?.let { { Text(it, style = MaterialTheme.typography.headlineSmall) } },
        text = { Column(verticalArrangement = Arrangement.spacedBy(Spacing.lg)) { content() } },
        confirmButton = {
            if (destructive) {
                Button(
                    onClick = onConfirm,
                    colors = ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.error,
                        contentColor = MaterialTheme.colorScheme.onError,
                    ),
                ) { Text(confirmText) }
            } else {
                Button(onClick = onConfirm) { Text(confirmText) }
            }
        },
        dismissButton = dismissText?.let { { TextButton(onClick = onDismiss) { Text(it) } } },
    )
}

/**
 * The consent prompt shared by camera, screen and control: heading, one line
 * on what is shared with which PC, then Deny and Allow, both plainly tappable
 * (Deny is tonal, not a bare text button). [onAlways] adds "Always allow".
 */
@Composable
fun ConsentDialog(
    title: String,
    body: String,
    onAllow: () -> Unit,
    onDeny: () -> Unit,
    onAlways: (() -> Unit)? = null,
) {
    androidx.compose.material3.AlertDialog(
        onDismissRequest = onDeny,
        shape = RoundedCornerShape(Radius.hero),
        title = { Text(title, style = MaterialTheme.typography.headlineSmall) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(Spacing.sm)) {
                Text(body, style = MaterialTheme.typography.bodyMedium)
                if (onAlways != null) TextButton(onClick = onAlways) { Text("Always allow") }
            }
        },
        dismissButton = { FilledTonalButton(onClick = onDeny) { Text("Deny") } },
        confirmButton = { Button(onClick = onAllow) { Text("Allow") } },
    )
}

/** A bordered rounded card on the surface colour, as used for the settings groups. */
@Composable
fun SectionCard(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    Card(
        shape = RoundedCornerShape(Radius.card),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        modifier = modifier.fillMaxWidth().padding(start = Spacing.lg, end = Spacing.lg, bottom = Spacing.md),
    ) {
        Column(Modifier.padding(vertical = Spacing.lg)) { content() }
    }
}
