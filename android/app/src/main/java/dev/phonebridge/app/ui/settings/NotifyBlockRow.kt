package dev.phonebridge.app.ui.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import dev.phonebridge.app.MirrorVerdict
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
    var open by remember { mutableStateOf(false) }
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
        fun label(pkg: String): String = runCatching {
            ctx.packageManager.getApplicationInfo(pkg, 0).loadLabel(ctx.packageManager).toString()
        }.getOrDefault(pkg)
        AlertDialog(
            onDismissRequest = { open = false },
            title = { Text("Apps on your PC") },
            text = {
                if (apps.isEmpty()) {
                    Text("Apps appear here once they post a notification. Blocked apps never leave this phone.")
                } else {
                    LazyColumn(Modifier.heightIn(max = 360.dp)) {
                        items(apps, key = { it }) { pkg ->
                            val sends = mirrorVerdict(pkg, false, blocked, allowed) == MirrorVerdict.Send
                            RelayRow(
                                title = label(pkg),
                                supporting = if (sends) "Shown on PC" else if (pkg !in blocked && blockedByDefault(pkg)) "Blocked by default" else "Blocked",
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
            },
            confirmButton = { TextButton(onClick = { open = false }) { Text("Done") } },
            dismissButton = {
                TextButton(onClick = { prefs.mirrorCounts = dev.phonebridge.app.MirrorCounts(); counts = prefs.mirrorCounts }) { Text("Reset counts") }
            },
        )
    }
}
