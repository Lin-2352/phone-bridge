package dev.phonebridge.app.ui.timeline

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.Uri
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.ui.graphics.Color
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import dev.phonebridge.app.ui.components.animationsEnabled
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
import androidx.compose.material.icons.automirrored.filled.VolumeUp
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
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.draw.clip
import dev.phonebridge.app.Radius
import dev.phonebridge.app.ui.components.EmptyState
import dev.phonebridge.app.ui.components.IconBadge
import dev.phonebridge.app.ui.components.RelayCard
import dev.phonebridge.app.ui.components.RelayRow
import dev.phonebridge.app.ui.components.RelayTint
import dev.phonebridge.app.ui.components.RelayTopBar
import dev.phonebridge.app.ui.components.SectionHeader
import dev.phonebridge.app.ui.components.cardColor
import dev.phonebridge.app.ui.components.tintColor
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
private class Day(val label: String, val isToday: Boolean, val key: String, val dayKey: String, val rows: List<Pair<String, TlEntry>>)

private fun days(entries: List<TlEntry>, now: Long, zone: ZoneId): List<Day> {
    val seen = HashMap<String, Int>()
    val seenDays = HashMap<String, Int>()
    return groupByDay(entries, now, zone).map { (label, group) ->
        Day(
            label,
            label == "Today",
            // Unique even if a day repeats (clock went backwards).
            "h:$label#${seenDays.merge(label, 1, Int::plus)}",
            // What the user opened or closed is remembered per calendar day, so a new "Today" starts fresh.
            dayKey(group.first().at, zone),
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
    // What the user opened or closed, by day label; survives rotation. Days not in it follow the default rule.
    var overrides by rememberSaveable(
        stateSaver = listSaver<Map<String, Boolean>, String>(
            save = { m -> m.flatMap { listOf(it.key, it.value.toString()) } },
            restore = { l -> l.chunked(2).associate { it[0] to (it[1] == "true") } },
        ),
    ) { mutableStateOf(emptyMap<String, Boolean>()) }
    val animate = remember { animationsEnabled(ctx) }
    val grouped = remember(entries, dayTick) { days(entries, System.currentTimeMillis(), ZoneId.systemDefault()) }
    var details by remember { mutableStateOf<TlEntry?>(null) }

    fun expandedOf(d: Day, count: Int) = isDayExpanded(d.dayKey, d.isToday, count, overrides)

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

    fun say(text: String, action: String? = null, onAction: () -> Unit = {}) {
        scope.launch {
            snack.currentSnackbarData?.dismiss()
            if (snack.showSnackbar(text, action, duration = androidx.compose.material3.SnackbarDuration.Long) == androidx.compose.material3.SnackbarResult.ActionPerformed) onAction()
        }
    }

    fun share(e: TlEntry) {
        val uri = e.uri ?: return say("This file cannot be shared again")
        if (opening) return
        opening = true
        scope.launch {
            val msg = try { shareSavedFile(ctx, uri) } finally { opening = false }
            msg?.let { snack.showSnackbar(it) }
        }
    }

    fun tap(e: TlEntry) {
        if (isOpenable(e)) open(e) else say(tapHint(e) ?: "")
    }

    fun act(e: TlEntry, a: EntryAction) {
        when (a) {
            EntryAction.Open -> open(e)
            EntryAction.ShareAgain -> share(e)
            EntryAction.CopyName -> {
                val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                cm.setPrimaryClip(android.content.ClipData.newPlainText("file name", e.name.orEmpty()))
                say("File name copied")
            }
            EntryAction.Details -> details = e
            EntryAction.Delete -> if (Timeline.remove(e)) say("Removed from history", "Undo") { Timeline.restore(e) }
        }
    }

    Scaffold(
        topBar = {
            RelayTopBar(
                title = "Timeline",
                actions = {
                    IconButton(onClick = { menu = true }) { Icon(Icons.Filled.MoreVert, "More") }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        DropdownMenuItem(
                            text = { Text("Clear history") },
                            enabled = entries.isNotEmpty(),
                            onClick = { menu = false; confirmClear = true },
                        )
                        val allOpen = allDaysExpanded(grouped.map { it.dayKey to it.isToday }, overrides)
                        DropdownMenuItem(
                            text = { Text(if (allOpen) "Collapse all" else "Expand all") },
                            enabled = grouped.isNotEmpty(),
                            onClick = { menu = false; overrides = setAllDays(grouped.map { it.dayKey }, !allOpen) },
                        )
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snack) },
    ) { pad ->
        LazyColumn(Modifier.padding(pad).fillMaxSize()) {
            item(key = "keep") {
                RelayCard {
                    RelayRow(
                        title = "Keep history",
                        supporting = "Remember what you send and receive here",
                        checked = historyOn,
                        onChecked = { on ->
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
                    EmptyState(emptyText(historyOn))
                }
            }
            for (day in grouped) {
                val isOpen = expandedOf(day, grouped.size)
                stickyHeader(key = day.key, contentType = "header") {
                    DayHeader(day.label, day.rows.size, isOpen, animate) { overrides = toggleDay(overrides, day.dayKey, isOpen) }
                }
                if (isOpen) {
                    items(day.rows, key = { it.first }, contentType = { "row" }) { (_, e) ->
                        Box(if (animate) Modifier.animateItem() else Modifier) { EntryRow(e, zone, onTap = { tap(e) }, onAction = { act(e, it) }) }
                    }
                }
            }
        }
    }

    details?.let { e ->
        RelayDialog(
            title = "Details",
            onDismissRequest = { details = null },
            confirmText = "Close",
            onConfirm = { details = null },
        ) {
            detailLines(e, zone).forEach { (k, v) ->
                Column {
                    Text(k, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text(v, style = MaterialTheme.typography.bodyMedium)
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
private fun DayHeader(label: String, count: Int, expanded: Boolean, animate: Boolean, onToggle: () -> Unit) {
    val turn by animateFloatAsState(if (expanded) 0f else -90f, tween(if (animate) 150 else 0), label = "day chevron")
    // Opaque, so rows scroll under it while it sticks. The whole row is the 48 dp target.
    Surface(Modifier.fillMaxWidth(), color = MaterialTheme.colorScheme.surface) {
        Row(
            Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp)
                .clickable(role = Role.Button, onClick = onToggle)
                .semantics(mergeDescendants = true) { stateDescription = if (expanded) "Expanded" else "Collapsed" }
                .padding(start = 24.dp, end = 24.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(Icons.Filled.KeyboardArrowDown, null, Modifier.size(20.dp).rotate(turn), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(label, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
            Text(itemCountText(count), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

private fun kindIcon(k: TlKind) = when (k) {
    TlKind.File -> Icons.AutoMirrored.Filled.InsertDriveFile
    TlKind.ClipText -> Icons.Filled.ContentPaste
    TlKind.ClipImage -> Icons.Filled.Image
    TlKind.Link -> Icons.Filled.Link
    TlKind.Notification -> Icons.Filled.Notifications
    TlKind.Audio -> Icons.AutoMirrored.Filled.VolumeUp
}

private fun kindTint(k: TlKind) = when (k) {
    TlKind.File, TlKind.Link -> RelayTint.Primary
    TlKind.ClipText -> RelayTint.Accent
    TlKind.ClipImage -> RelayTint.Success
    TlKind.Notification -> RelayTint.Warning
    TlKind.Audio -> RelayTint.Accent
}

private fun actionLabel(a: EntryAction) = when (a) {
    EntryAction.Open -> "Open"
    EntryAction.ShareAgain -> "Share again"
    EntryAction.CopyName -> "Copy file name"
    EntryAction.Details -> "Details"
    EntryAction.Delete -> "Delete"
}

/**
 * One entry. A tap opens it when it can be opened and otherwise says why; long-press and the "more" button
 * open the same menu (the button is the one TalkBack and switch users can find).
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun EntryRow(e: TlEntry, zone: ZoneId, onTap: () -> Unit, onAction: (EntryAction) -> Unit) {
    var menu by remember { mutableStateOf(false) }
    val openable = isOpenable(e)
    Row(
        Modifier.fillMaxWidth()
            .padding(horizontal = Spacing.lg, vertical = Spacing.xs)
            .clip(RoundedCornerShape(Radius.card))
            .background(cardColor())
            .combinedClickable(
                onClickLabel = if (openable) "Open" else "Why this cannot be opened",
                onClick = onTap,
                onLongClickLabel = "Show actions",
                onLongClick = { menu = true },
            )
            .padding(start = Spacing.lg, top = Spacing.xs, bottom = Spacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.md),
    ) {
        IconBadge(kindIcon(e.kind), tintColor(kindTint(e.kind)))
        Column(Modifier.weight(1f).padding(vertical = Spacing.xs)) {
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
        Box {
            IconButton(onClick = { menu = true }, modifier = Modifier.size(48.dp)) {
                Icon(Icons.Filled.MoreVert, "Actions for ${summary(e)}")
            }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                entryActions(e).forEach { a ->
                    DropdownMenuItem(
                        text = { Text(actionLabel(a), color = if (a == EntryAction.Delete) MaterialTheme.colorScheme.error else Color.Unspecified) },
                        onClick = { menu = false; onAction(a) },
                    )
                }
            }
        }
    }
}
