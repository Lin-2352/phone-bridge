package dev.phonebridge.app.ui.settings

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import dev.phonebridge.app.MirrorVerdict
import dev.phonebridge.app.RelayDialog
import dev.phonebridge.app.Prefs
import dev.phonebridge.app.blockedByDefault
import dev.phonebridge.app.mirrorVerdict
import dev.phonebridge.app.ui.LocalResumeCount
import dev.phonebridge.app.ui.components.RelayRow

/**
 * Settings, Notifications: which phone apps may show their notifications on the PC, with the running
 * count of mirrored vs blocked. Banking, authenticator and password apps start blocked (see
 * NotifyBlock.kt); the list holds apps seen recently plus the ones the user changed.
 */
@Composable
fun NotifyBlockRow() {
    val ctx = LocalContext.current
    val resumed = LocalResumeCount.current
    val prefs = remember { Prefs(ctx) }
    var open by rememberSaveable { mutableStateOf(false) }
    var blocked by remember(resumed, open) { mutableStateOf(prefs.notifyBlocked) }
    var allowed by remember(resumed, open) { mutableStateOf(prefs.notifyAllowed) }
    var counts by remember(resumed, open) { mutableStateOf(prefs.mirrorCounts) }
    val apps = remember(resumed, open) {
        (prefs.notifySeen + prefs.notifyBlocked + prefs.notifyAllowed).distinct()
    }
    val off = apps.count { mirrorVerdict(it, false, blocked, allowed) != MirrorVerdict.Send }
    RelayRow(
        title = "Apps on your PC",
        supporting = "${counts.summary()}. $off app${if (off == 1) "" else "s"} blocked. Banking and authenticator apps are blocked by default",
        onClick = { open = true },
    )
    if (open) {
        var query by remember { mutableStateOf("") }
        val rows = remember(apps) {
            orderAppRows(apps.map { pkg ->
                val label = runCatching { ctx.packageManager.getApplicationInfo(pkg, 0).loadLabel(ctx.packageManager).toString() }.getOrNull()
                AppRow(pkg, label ?: pkg, label != null)
            })
        }
        val shown = filterAppRows(rows, query)
        RelayDialog(
            title = "Apps on your PC",
            onDismissRequest = { open = false },
            confirmText = "Done",
            onConfirm = { open = false },
        ) {
            if (apps.isEmpty()) {
                Text("Apps appear here once they post a notification. Blocked apps never leave this phone.")
            } else {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    singleLine = true,
                    label = { Text("Search apps") },
                    modifier = Modifier.fillMaxWidth(),
                )
                if (shown.isEmpty()) {
                    Text("No app matches.", style = MaterialTheme.typography.bodyMedium)
                } else {
                    LazyColumn(Modifier.heightIn(max = 320.dp)) {
                        items(shown, key = { it.pkg }) { row ->
                            val pkg = row.pkg
                            val sends = mirrorVerdict(pkg, false, blocked, allowed) == MirrorVerdict.Send
                            RelayRow(
                                title = row.label,
                                supporting = (if (!row.installed) "Not installed. " else "") +
                                    if (sends) "Shown on PC" else if (pkg !in blocked && blockedByDefault(pkg)) "Blocked by default" else "Blocked",
                                checked = sends,
                                onChecked = { on ->
                                    if (on) {
                                        blocked = blocked - pkg
                                        if (blockedByDefault(pkg)) allowed = allowed + pkg
                                    } else {
                                        blocked = blocked + pkg
                                        allowed = allowed - pkg
                                    }
                                    prefs.notifyBlocked = blocked
                                    prefs.notifyAllowed = allowed
                                },
                            )
                        }
                    }
                }
                Text(counts.summary(), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                TextButton(
                    onClick = { prefs.mirrorCounts = dev.phonebridge.app.MirrorCounts(); counts = prefs.mirrorCounts },
                    modifier = Modifier.heightIn(min = 48.dp),
                ) { Text("Reset counts") }
            }
        }
    }
}
