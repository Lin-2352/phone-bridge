package dev.phonebridge.app.ui.components

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/** How a sheet was asked to close. Only [Choice] (a button) answers a consent. */
enum class DismissReason { Choice, OutsideTap, Drag, Back }

/**
 * A consent sheet answers only with a button: a tap outside, a drag or the back button never answers it for the
 * person, so a consent is never given or refused by accident.
 */
fun sheetDismissAllowed(reason: DismissReason): Boolean = reason == DismissReason.Choice

/**
 * The consent sheet: Allow and Deny, and "Always allow" when the request offers it. The guards stay where they
 * were (ConsentGuard, SecureWindow, hidden overlays); this sheet only changes how the question is shown.
 */
@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
fun ConsentSheet(
    title: String,
    body: String,
    onAllow: () -> Unit,
    onDeny: () -> Unit,
    onAlways: (() -> Unit)? = null,
) {
    // Dismissal by outside tap, drag or back is ignored: only a button answers.
    ModalBottomSheet(onDismissRequest = {}) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 16.dp)) {
            Text(title, style = MaterialTheme.typography.titleLarge)
            Text(body, style = MaterialTheme.typography.bodyMedium)
            Button(onClick = onAllow, modifier = Modifier.fillMaxWidth()) { Text("Allow") }
            if (onAlways != null) {
                OutlinedButton(onClick = onAlways, modifier = Modifier.fillMaxWidth()) { Text("Always allow") }
            }
            OutlinedButton(onClick = onDeny, modifier = Modifier.fillMaxWidth()) { Text("Deny") }
        }
    }
}
