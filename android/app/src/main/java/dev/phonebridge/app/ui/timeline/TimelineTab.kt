package dev.phonebridge.app.ui.timeline

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
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
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
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
import dev.phonebridge.app.RelayDialog
import dev.phonebridge.app.Spacing
import dev.phonebridge.app.TlDir
import dev.phonebridge.app.TlEntry
import dev.phonebridge.app.TlKind
import dev.phonebridge.app.Timeline
import dev.phonebridge.app.summary
import java.time.ZoneId
import kotlinx.coroutines.launch

/** A day with its entries; each entry has a key that is unique and survives new entries at the top. */
private class Day(val label: String, val key: String, val rows: List<Pair<String, TlEntry>>)

private fun days(entries: List<TlEntry>, now: Long, zone: ZoneId): List<Day> {
    val seen = HashMap<String, Int>()
    val seenDays = HashMap<String, Int>()
    return groupByDay(entries, now, zone).map { (label, group) ->
        Day(
            label,
            // Unique even if a day repeats (clock went backwards).
            "h:$label#${seenDays.merge(label, 1, Int::plus)}",
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
    val historyOn by Timeline.historyOn.collectAsState()
    var confirmOff by remember { mutableStateOf(false) }
    var confirmClear by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    // Bumped when the date, time or time zone changes, and on resume, so "Today" and the clock times are redrawn.
    var dayTick by remember { mutableIntStateOf(0) }
    var opening by remember { mutableStateOf(false) }
    DisposableEffect(ctx) {
        val r = object : BroadcastReceiver() {
            override fun onReceive(c: Context?, i: Intent?) { dayTick++ }
        }
        val f = IntentFilter().apply {
            addAction(Intent.ACTION_DATE_CHANGED)
            addAction(Intent.ACTION_TIMEZONE_CHANGED)
            addAction(Intent.ACTION_TIME_CHANGED)
        }
        ctx.registerReceiver(r, f)
        onDispose { ctx.unregisterReceiver(r) }
    }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle) {
        val o = LifecycleEventObserver { _, ev -> if (ev == Lifecycle.Event.ON_RESUME) dayTick++ }
        lifecycle.addObserver(o)
        onDispose { lifecycle.removeObserver(o) }
    }
    val zone = ZoneId.systemDefault()
    val grouped = remember(entries, dayTick) { days(entries, System.currentTimeMillis(), ZoneId.systemDefault()) }

    // The check touches storage, so it runs off the main thread, and only on a tap.
    fun open(e: TlEntry) {
        val uri = e.uri ?: return
        if (!isOpenable(e) || opening) return
        opening = true // one tap, one viewer or one message
        scope.launch {
            val msg: String? = try {
                openSavedFile(ctx, uri)
            } finally {
                opening = false
            }
            msg?.let { snack.showSnackbar(it) }
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
    val openable = isOpenable(e)
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
