package dev.phonebridge.app.ui.timeline

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.phonebridge.app.Prefs
import dev.phonebridge.app.RelayDialog
import dev.phonebridge.app.Spacing
import dev.phonebridge.app.TlDir
import dev.phonebridge.app.TlEntry
import dev.phonebridge.app.TlKind
import dev.phonebridge.app.Timeline
import dev.phonebridge.app.fileState
import dev.phonebridge.app.summary
import java.time.ZoneId
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A day with its entries; each entry has a key that is unique and survives new entries at the top. */
private class Day(val label: String, val key: String, val rows: List<Pair<String, TlEntry>>)

private fun days(entries: List<TlEntry>, now: Long, zone: ZoneId): List<Day> {
    val seen = HashMap<String, Int>()
    return groupByDay(entries, now, zone).mapIndexed { i, (label, group) ->
        Day(
            label,
            "h:$label:${group.first().at}:$i",
            group.map { e ->
                val base = "${e.at}|${e.kind}|${e.dir}|${e.peer}|${e.name ?: e.app}"
                "e:$base#${seen.merge(base, 1, Int::plus)}" to e
            },
        )
    }
}

@Composable
@OptIn(ExperimentalFoundationApi::class, androidx.compose.material3.ExperimentalMaterial3Api::class)
fun TimelineTab() {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val snack = remember { SnackbarHostState() }
    val entries by Timeline.entries.collectAsState()
    var historyOn by remember { mutableStateOf(Prefs(ctx).keepHistory) }
    var confirmOff by remember { mutableStateOf(false) }
    var confirmClear by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    val zone = remember(entries) { ZoneId.systemDefault() }
    val grouped = remember(entries) { days(entries, System.currentTimeMillis(), zone) }

    // The check touches storage, so it runs off the main thread, and only on a tap.
    fun open(e: TlEntry) {
        val uri = e.uri ?: return
        scope.launch {
            val present = withContext(Dispatchers.IO) { fileState(ctx, uri) }
            if (!present) {
                snack.showSnackbar("File moved or deleted")
                return@launch
            }
            val u = Uri.parse(uri)
            val type = withContext(Dispatchers.IO) { runCatching { ctx.contentResolver.getType(u) }.getOrNull() } ?: "*/*"
            val view = Intent(Intent.ACTION_VIEW).setDataAndType(u, type).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            try {
                ctx.startActivity(view)
            } catch (x: ActivityNotFoundException) {
                snack.showSnackbar("No app can open this file")
            } catch (x: SecurityException) {
                snack.showSnackbar("File moved or deleted")
            }
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Timeline") },
                actions = {
                    IconButton(onClick = { menu = true }) { Icon(Icons.Filled.MoreVert, "More") }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        DropdownMenuItem(
                            text = { Text("Clear history") },
                            enabled = entries.isNotEmpty(),
                            onClick = { menu = false; confirmClear = true },
                        )
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snack) },
    ) { pad ->
        LazyColumn(Modifier.padding(pad).fillMaxSize()) {
            item(key = "keep") {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = Spacing.lg, vertical = Spacing.sm),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text("Keep history", style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                    Switch(
                        checked = historyOn,
                        onCheckedChange = { on ->
                            if (on) {
                                Timeline.setEnabled(true)
                                historyOn = true
                            } else {
                                confirmOff = true // stays on until confirmed
                            }
                        },
                    )
                }
            }
            if (grouped.isEmpty()) {
                item(key = "empty") {
                    Box(Modifier.fillMaxWidth().padding(Spacing.xl), contentAlignment = Alignment.Center) {
                        Text(
                            emptyText(historyOn),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            for (day in grouped) {
                stickyHeader(key = day.key, contentType = "header") { DayHeader(day.label) }
                items(day.rows, key = { it.first }, contentType = { "row" }) { (_, e) ->
                    EntryRow(e, zone) { open(e) }
                }
            }
        }
    }

    if (confirmOff) {
        RelayDialog(
            title = "Turn off history?",
            onDismissRequest = { confirmOff = false },
            confirmText = "Turn off",
            onConfirm = {
                Timeline.setEnabled(false)
                historyOn = false
                confirmOff = false
            },
            dismissText = "Cancel",
            destructive = true,
        ) { Text("This deletes the history on this phone.") }
    }
    if (confirmClear) {
        RelayDialog(
            title = "Clear history?",
            onDismissRequest = { confirmClear = false },
            confirmText = "Clear",
            onConfirm = {
                Timeline.clear()
                confirmClear = false
            },
            dismissText = "Cancel",
            destructive = true,
        ) { Text("This deletes the history on this phone.") }
    }
}

@Composable
private fun DayHeader(label: String) {
    // Opaque, so rows scroll under it while it sticks.
    Surface(Modifier.fillMaxWidth(), color = MaterialTheme.colorScheme.surface) {
        Text(
            label,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(horizontal = Spacing.lg, vertical = Spacing.sm),
        )
    }
}

private fun kindIcon(k: TlKind) = when (k) {
    TlKind.File -> Icons.AutoMirrored.Filled.InsertDriveFile
    TlKind.ClipText -> Icons.Filled.ContentPaste
    TlKind.ClipImage -> Icons.Filled.Image
    TlKind.Link -> Icons.Filled.Link
    TlKind.Notification -> Icons.Filled.Notifications
}

@Composable
private fun EntryRow(e: TlEntry, zone: ZoneId, onOpen: () -> Unit) {
    // A sent file has no uri (the share permission was temporary): not openable, which is not "moved or deleted".
    val openable = e.kind == TlKind.File && e.uri != null
    Row(
        Modifier.fillMaxWidth()
            .then(if (openable) Modifier.clickable(onClick = onOpen) else Modifier)
            .padding(horizontal = Spacing.lg, vertical = Spacing.md),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.md),
    ) {
        Icon(kindIcon(e.kind), null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Column(Modifier.weight(1f)) {
            Text(summary(e), style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    if (e.dir == TlDir.ToPc) Icons.Filled.ArrowUpward else Icons.Filled.ArrowDownward,
                    if (e.dir == TlDir.ToPc) "Sent to" else "From",
                    Modifier.size(14.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    e.peer,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(start = Spacing.xs),
                )
            }
        }
        Text(clockText(e.at, zone), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
