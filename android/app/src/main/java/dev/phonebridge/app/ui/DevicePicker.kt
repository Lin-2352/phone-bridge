package dev.phonebridge.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uniffi.pb_ffi.PbPeer

/** One PC in the picker: its label (a fingerprint suffix when names repeat), its status line and its flags. */
data class PickerRow(
    val id: String,
    val label: String,
    val status: String,
    val isDefault: Boolean,
    val live: Boolean,
)

/**
 * The picker's rows: the default PC first, then the connected ones, then the rest, each group by name ignoring
 * case. Two PCs with the same name get a suffix of the first four characters of their fingerprint.
 */
fun pickerRows(
    peers: List<PbPeer>,
    live: Set<String>,
    lastSeen: Map<String, Long>,
    defaultId: String?,
    nowMs: Long,
): List<PickerRow> {
    val repeated = peers.groupBy { it.name }.filterValues { it.size > 1 }.keys
    return peers
        .map { p ->
            val label = if (p.name in repeated) "${p.name} (${p.fingerprint.take(4)})" else p.name
            val isLive = p.id in live
            PickerRow(
                id = p.id,
                label = label,
                status = when {
                    isLive -> "Connected"
                    lastSeen[p.id] == null -> "Not seen yet"
                    else -> "Last seen " + ago(nowMs - lastSeen.getValue(p.id))
                },
                isDefault = p.id == defaultId,
                live = isLive,
            )
        }
        .sortedWith(
            compareBy<PickerRow>({ !it.isDefault }, { !it.live }, { it.label.lowercase() }),
        )
}

private fun ago(ms: Long): String {
    val minutes = ms / 60_000
    return when {
        minutes < 1 -> "just now"
        minutes < 60 -> "$minutes min ago"
        minutes < 24 * 60 -> "${minutes / 60} h ago"
        else -> "${minutes / (24 * 60)} d ago"
    }
}

/** The picker sheet: tap a PC to send to it. Dismissing it sends nothing. */
@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
fun DevicePickerSheet(
    rows: List<PickerRow>,
    onPick: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
            rows.forEach { row ->
                Column(
                    Modifier
                        .fillMaxWidth()
                        .clickable { onPick(row.id) }
                        .padding(horizontal = 24.dp, vertical = 12.dp),
                ) {
                    Text(if (row.isDefault) "${row.label} (default)" else row.label)
                    Text(row.status)
                }
            }
        }
    }
}
