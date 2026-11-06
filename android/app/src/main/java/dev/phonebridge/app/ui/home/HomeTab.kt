package dev.phonebridge.app.ui.home

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Cameraswitch
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.automirrored.filled.ScreenShare
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.TouchApp
import androidx.compose.material.icons.filled.UploadFile
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.foundation.layout.Spacer
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.material3.ListItem
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.filled.Link
import androidx.compose.material3.ListItemDefaults
import androidx.compose.ui.graphics.Color
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.phonebridge.app.APP_NAME
import dev.phonebridge.app.AudioShare
import dev.phonebridge.app.audio.AudioDirection
import dev.phonebridge.app.Bridge
import dev.phonebridge.app.notifyInfo
import dev.phonebridge.app.heroPc
import dev.phonebridge.app.CameraShare
import dev.phonebridge.app.ClipImages
import dev.phonebridge.app.ControlService
import dev.phonebridge.app.PbService
import dev.phonebridge.app.Prefs
import dev.phonebridge.app.Quick
import dev.phonebridge.app.QuickTile
import dev.phonebridge.app.Radius
import dev.phonebridge.app.ScanActivity
import dev.phonebridge.app.tileDisabledReason
import dev.phonebridge.app.ScreenShare
import dev.phonebridge.app.ScreenshotWatcher
import dev.phonebridge.app.SendClipboardActivity
import dev.phonebridge.app.Spacing
import dev.phonebridge.app.Timeline
import dev.phonebridge.app.TouchpadActivity
import dev.phonebridge.app.hasNotifyAccess
import dev.phonebridge.app.notifyAccessMissing
import dev.phonebridge.app.pruneAllowed
import dev.phonebridge.app.liveTileText
import dev.phonebridge.app.quickTiles
import dev.phonebridge.app.startsOnPcText
import dev.phonebridge.app.setBridgeEnabled
import dev.phonebridge.app.ui.LocalResumeCount
import dev.phonebridge.app.ui.components.IconBadge
import dev.phonebridge.app.ui.components.RelayTint
import dev.phonebridge.app.ui.components.RelayTopBar
import dev.phonebridge.app.ui.components.cardColor
import dev.phonebridge.app.ui.components.tintColor
import dev.phonebridge.app.ui.components.LinkIndicator
import dev.phonebridge.app.RelayDialog
import dev.phonebridge.app.ui.timeline.RecentItem
import dev.phonebridge.app.ui.timeline.isOpenable
import dev.phonebridge.app.ui.timeline.openSavedFile
import dev.phonebridge.app.ui.timeline.recentItem
import dev.phonebridge.app.widget.sendPickedFiles
import dev.phonebridge.app.widget.widgetConnected
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * The Home tab: the PC hero card, one "Needs attention" card (hidden when nothing needs it),
 * the 2 x 3 quick actions, a card with Stop for each live session, and the last few Recent items.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeTab(onTimeline: () -> Unit) {
    val ctx = LocalContext.current
    val resumed = LocalResumeCount.current
    val prefs = remember { Prefs(ctx) }
    val linkedPc = Bridge.linked.value
    val running = Bridge.running.value
    val ready = Bridge.nodeReady.value
    // Remember the PC while it is live; forget it only once the running node says nothing is paired.
    LaunchedEffect(linkedPc, running, ready) {
        if (linkedPc != null) { if (prefs.lastPcName != linkedPc) prefs.lastPcName = linkedPc }
        else if (running && ready && prefs.lastPcName != null) prefs.lastPcName = null
    }
    val shown = heroPc(linkedPc, prefs.lastPcName, running, ready)
    val pc = linkedPc
    val controller = Bridge.controlledBy.value
    val screenTo = ScreenShare.sharingWith.value
    val cameraTo = CameraShare.usedBy.value
    val audioOut = AudioShare.sharingWith.value
    val audioIn = AudioShare.playingFrom.value
    // Live, as on the PC: only while the PC really has a session open with this phone.
    val live = widgetConnected(running, pc, Bridge.liveSessions.intValue, controller, screenTo, cameraTo, audioOut ?: audioIn)
    val storedSeen = remember(resumed) { prefs.lastSeenMs }
    val lastSeen = maxOf(Bridge.lastSeen.longValue, storedSeen)
    // "Last seen 5 min ago" moves on, and the node-side needs are re-read, while Home is on screen
    // (stopped with the activity: no polling in the background).
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                now = System.currentTimeMillis()
                delay(5_000)
            }
        }
    }

    val list = remember(resumed, now) {
        val node = Bridge.node
        // Running, not only turned on: Android can leave it enabled but stopped.
        val controlOn = ControlService.instance != null
        // A node shut down meanwhile throws: then nothing is waiting or forgotten.
        val refusedAt = node?.let { runCatching { it.controlRefusedAt().toLong() }.getOrNull() } ?: 0L
        val forgot = node?.let { n -> runCatching { n.peers().any { n.peerChanged(it.id) } }.getOrNull() } == true
        needs(
            notifyAccess = !notifyAccessMissing(prefs.notifyToPc, hasNotifyAccess(ctx)),
            screenshotsOn = prefs.shareScreenshots,
            photosGranted = ClipImages.hasImageAccess(ctx),
            controlOn = controlOn,
            controlWaiting = controlWaiting(refusedAt, now, controlOn),
            pcForgot = forgot,
            appNotificationsOn = ctx.getSystemService(android.app.NotificationManager::class.java).areNotificationsEnabled(),
            identityLost = prefs.identityLost,
        )
    }
    var confirmReset by remember { mutableStateOf(false) }
    if (confirmReset) {
        val prompt = identityResetPrompt()
        RelayDialog(
            title = prompt.title,
            onDismissRequest = { confirmReset = false },
            confirmText = "Reset and pair again",
            destructive = true,
            onConfirm = {
                confirmReset = false
                dev.phonebridge.app.resetIdentity(ctx)
            },
            dismissText = "Cancel",
        ) {
            Text(prompt.body, style = MaterialTheme.typography.bodyMedium)
        }
    }
    var forgetNames by remember { mutableStateOf<List<String>?>(null) }
    forgetNames?.let { names ->
        val prompt = forgetPrompt(names)
        RelayDialog(
            title = prompt.title,
            onDismissRequest = { forgetNames = null },
            confirmText = "Remove and pair again",
            destructive = true,
            onConfirm = {
                forgetNames = null
                // Re-reads which PCs are flagged at this moment.
                removeForgottenPcs(ctx)
                ctx.startActivity(Intent(ctx, ScanActivity::class.java))
            },
            dismissText = "Cancel",
        ) {
            Text(prompt.body, style = MaterialTheme.typography.bodyMedium)
        }
    }
    val snack = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    var opening by remember { mutableStateOf(false) }
    val accessOpener = dev.phonebridge.app.ui.settings.rememberAccessOpener()
    val askPhotos = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { _ ->
        ScreenshotWatcher.sync(ctx)
        val access = ClipImages.imageAccess(ctx)
        val granted = access == dev.phonebridge.app.ImageAccess.FULL
        // Picked photos only: Android's settings page has "Allow all".
        if (access == dev.phonebridge.app.ImageAccess.PARTIAL) {
            android.widget.Toast.makeText(ctx, dev.phonebridge.app.PARTIAL_PHOTOS_TEXT, android.widget.Toast.LENGTH_LONG).show()
            openAppDetails(ctx)
            return@rememberLauncherForActivityResult
        }
        // Android no longer shows its prompt once the user said "don't ask again": the app's settings page.
        val act = ctx as? Activity
        if (!granted && act != null && !act.shouldShowRequestPermissionRationale(ClipImages.imagePermission)) openAppDetails(ctx)
    }
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) sendPickedFiles(ctx, uris)
    }

    val timeline by Timeline.entries.collectAsState()
    val historyOn by Timeline.historyOn.collectAsState()
    val recent = remember(timeline) { timeline.take(3).map(::recentItem) }
    Scaffold(
        snackbarHost = { SnackbarHost(snack) },
        topBar = {
            // No Settings button here: the bottom bar's Settings tab is the one way in.
            RelayTopBar(title = APP_NAME)
        },
    ) { pad ->
        LazyColumn(
            Modifier.padding(pad).padding(horizontal = Spacing.lg),
            verticalArrangement = Arrangement.spacedBy(Spacing.md),
        ) {
            item(key = "hero") { HeroCard(shown.name, shown.dim, running, ready, live, pcStatus(live, lastSeen, now, paired = pc != null)) }
            if (list.isNotEmpty()) {
                item(key = "needs") {
                    NeedsCard(list, Modifier.animateItem()) { need ->
                        when (need) {
                            Need.IdentityLost -> confirmReset = true
                            Need.NotificationAccess -> accessOpener(dev.phonebridge.app.ui.settings.SpecialAccess.Notifications)
                            Need.PhotosPermission -> askPhotos.launch(ClipImages.imagePermissions)
                            Need.ControlOff -> accessOpener(dev.phonebridge.app.ui.settings.SpecialAccess.Control)
                            Need.PcForgotPhone -> forgetNames = forgottenPcNames()
                            Need.AppNotificationsOff -> openSettings(
                                ctx,
                                Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, ctx.packageName),
                            )
                        }
                    }
                }
            }
            item(key = "grid") {
                QuickGrid(quickTiles(running, pc != null, cameraTo != null, screenTo != null, audioOut != null, audioIn != null), running, pc != null, { scope.launch { snack.showSnackbar(it) } }) { q ->
                    fun liveTile(name: String) { scope.launch { snack.showSnackbar(liveTileText(name)) } }
                    when (q) {
                        Quick.SendFile -> pickFiles.launch(arrayOf("*/*"))
                        // The activity reads the clipboard once it has focus (Android's rule), then closes.
                        Quick.SendClipboard -> ctx.startActivity(Intent(ctx, SendClipboardActivity::class.java))
                        Quick.Screen -> if (screenTo != null) liveTile("Screen") else startsOnPc(ctx, "Screen")
                        Quick.Camera -> if (cameraTo != null) liveTile("Camera") else startsOnPc(ctx, "Camera")
                        Quick.PhoneAudio -> if (audioOut != null) liveTile("Phone audio") else startsOnPc(ctx, "Phone audio")
                        Quick.PcAudio -> if (audioIn != null) liveTile("PC audio") else startsOnPc(ctx, "PC audio")
                        Quick.Touchpad -> ctx.startActivity(Intent(ctx, TouchpadActivity::class.java))
                        Quick.Pair -> ctx.startActivity(Intent(ctx, ScanActivity::class.java))
                    }
                }
            }
            controller?.let { item(key = "session-control") { SessionCard("$it is controlling this phone", "Its mouse and keyboard work here", Modifier.animateItem()) { Bridge.node?.stopControl() } } }
            screenTo?.let { item(key = "session-screen") { SessionCard("Sharing your screen with $it", "Everything on this screen shows on your PC", Modifier.animateItem()) { ScreenShare.stop(userStopped = true) } } }
            cameraTo?.let { item(key = "session-camera") { SessionCard("$it is using the camera", "This phone is your PC's webcam", Modifier.animateItem()) { CameraShare.stop(userStopped = true) } } }
            audioOut?.let { item(key = "session-audio-out") { SessionCard(dev.phonebridge.app.audio.audioShareCard(AudioShare.sharingSource.value, it).first, dev.phonebridge.app.audio.audioShareCard(AudioShare.sharingSource.value, it).second, Modifier.animateItem()) { AudioShare.stop(AudioDirection.PHONE_TO_PC) } } }
            audioIn?.let { item(key = "session-audio-in") { SessionCard("Playing audio from $it", "Your PC's sound plays on this phone", Modifier.animateItem()) { AudioShare.stop(AudioDirection.PC_TO_PHONE) } } }
            item(key = "recent-head") {
                Row(Modifier.fillMaxWidth().padding(top = Spacing.sm), verticalAlignment = Alignment.CenterVertically) {
                    Text("Recent", color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f).padding(start = Spacing.lg))
                    if (recent.isNotEmpty()) TextButton(onClick = onTimeline) { Text("See all") }
                }
            }
            if (recent.isEmpty()) {
                item(key = "recent-empty") { Text(if (historyOn) "Nothing yet. Share a photo or file to $APP_NAME from any app." else "History is off, so nothing is listed here.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = Spacing.sm, vertical = Spacing.sm)) }
            }
            items(recent) { r ->
                ListItem(
                    headlineContent = { Text(r.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    supportingContent = { Text(r.subtitle) },
                    leadingContent = {
                        when (r.kind) {
                            RecentItem.Kind.File -> IconBadge(Icons.AutoMirrored.Filled.InsertDriveFile, tintColor(RelayTint.Primary))
                            RecentItem.Kind.Clipboard -> IconBadge(Icons.Filled.ContentPaste, tintColor(RelayTint.Accent))
                            RecentItem.Kind.Link -> IconBadge(Icons.Filled.Link, tintColor(RelayTint.Primary))
                            RecentItem.Kind.Picture -> IconBadge(Icons.Filled.Image, tintColor(RelayTint.Success))
                            RecentItem.Kind.Notification -> IconBadge(Icons.Filled.Notifications, tintColor(RelayTint.Warning))
                            RecentItem.Kind.Audio -> IconBadge(Icons.AutoMirrored.Filled.VolumeUp, tintColor(RelayTint.Accent))
                        }
                    },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    modifier = Modifier.clip(RoundedCornerShape(Radius.card)).background(cardColor()).clickable(onClickLabel = if (r.hint == null) "Open" else "Why this cannot be opened") {
                        val uri = r.uri
                        if (!isOpenable(r.kind == RecentItem.Kind.File, uri)) {
                            r.hint?.let { h -> scope.launch { snack.showSnackbar(h) } }
                        } else if (uri != null && !opening) {
                            opening = true // one tap, one viewer or one message, as in Timeline
                            scope.launch {
                                val msg = try { openSavedFile(ctx, uri) } finally { opening = false }
                                msg?.let { snack.showSnackbar(it) }
                            }
                        }
                    },
                )
            }
        }
    }
}

/** The PC hero card: both devices joined by the link line, the PC's name and Connected / Last seen. */
@Composable
private fun HeroCard(pc: String?, pcDim: Boolean, running: Boolean, ready: Boolean, live: Boolean, status: String) {
    val ctx = LocalContext.current
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Radius.hero))
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .animateContentSize()
            .padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        LinkIndicator(Build.MODEL ?: "Phone", pc, live && running, pcDim)
        when {
            !running -> {
                HeroText("$APP_NAME is off", "Your PC cannot reach this phone.")
                Button(onClick = { setBridgeEnabled(ctx, true) }, modifier = Modifier.semantics { contentDescription = "Turn on $APP_NAME" }) { Text("Turn on") }
            }
            !ready -> HeroText("Starting…", null)
            pc == null -> {
                HeroText("Not paired yet", "On your PC, open $APP_NAME and choose Pair new device, then tap Pair.")
                Button(onClick = { ctx.startActivity(Intent(ctx, ScanActivity::class.java)) }) { Text("Pair") }
            }
            else -> HeroText(pc, status, accent = live)
        }
    }
}

@Composable
private fun HeroText(title: String, body: String?, accent: Boolean = false) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(title, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (body != null) {
            Text(body, style = MaterialTheme.typography.bodyMedium, color = if (accent) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** The single "Needs attention" card: one row per [needs] entry, each with its fix. */
@Composable
private fun NeedsCard(needs: List<Need>, modifier: Modifier, onFix: (Need) -> Unit) {
    Card(
        shape = RoundedCornerShape(Radius.card),
        colors = CardDefaults.cardColors(containerColor = needsContainer(needs), contentColor = needsContent(needs)),
        modifier = modifier.fillMaxWidth().animateContentSize(),
    ) {
        Column(Modifier.padding(vertical = Spacing.md)) {
            Text("Needs attention", style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(horizontal = Spacing.lg))
            needs.forEach { need ->
                val (title, body, fix) = needText(need)
                NeedRow(title, body, fix) { onFix(need) }
            }
        }
    }
}

/** Heading, explanation and fix button for one [Need]. */
private fun needText(need: Need): Triple<String, String, String> = when (need) {
    Need.IdentityLost -> Triple(
        "$APP_NAME can't open its keys",
        "This phone's identity can't be read any more, usually after a change of screen lock. Reset it, then pair again with your PC.",
        "Reset and pair again",
    )
    Need.NotificationAccess -> Triple(
        "Phone notifications are not reaching your PC",
        "$APP_NAME needs notification access. Tap Fix, then turn on $APP_NAME.",
        "Fix",
    )
    Need.PhotosPermission -> Triple(
        "Screenshots are not reaching your PC",
        "$APP_NAME needs access to your photos to see new screenshots.",
        "Allow",
    )
    Need.ControlOff -> Triple(
        "Your PC cannot control this phone",
        "To use your PC's mouse and keyboard here, turn on “$APP_NAME control” in Accessibility settings. " +
            "It switches itself off after each session, because banking apps refuse to open while it is on.",
        "Turn on",
    )
    Need.PcForgotPhone -> Triple(
        "Your PC no longer recognises this phone",
        "Something answering as your PC has a different identity. This happens if $APP_NAME on the PC was reset or reinstalled " +
            "— or if another device on this network is interfering. If you recognise the reset, remove the old pairing here and pair again.",
        "Remove and pair again",
    )
    Need.AppNotificationsOff -> Triple(
        "Notifications are off",
        "You will not see files, links or clips arriving from your PC.",
        "Turn on",
    )
}

@Composable
private fun NeedRow(title: String, body: String, fix: String, onFix: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = Spacing.lg, vertical = Spacing.sm), verticalArrangement = Arrangement.spacedBy(Spacing.xs)) {
        Text(title, style = MaterialTheme.typography.bodyLarge)
        Text(body, style = MaterialTheme.typography.bodyMedium, color = LocalContentColor.current.copy(alpha = 0.8f))
        FilledTonalButton(onClick = onFix, modifier = Modifier.align(Alignment.End).semantics { contentDescription = "$fix. $title" }) { Text(fix) }
    }
}

/** The quick actions, two per row; a tap gives a haptic tick. */
@Composable
private fun QuickGrid(tiles: List<QuickTile>, running: Boolean, paired: Boolean, say: (String) -> Unit, onTap: (Quick) -> Unit) {
    val haptics = LocalHapticFeedback.current
    Column(verticalArrangement = Arrangement.spacedBy(Spacing.md)) {
        tiles.chunked(2).forEach { row ->
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(Spacing.md)) {
                row.forEach { tile ->
                    QuickCard(tile, Modifier.weight(1f)) {
                        haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                        // A dimmed tile still answers: it says what is missing instead of doing nothing.
                        if (tile.enabled) onTap(tile.quick) else say(tileDisabledReason(running, paired))
                    }
                }
                if (row.size == 1) Spacer(Modifier.weight(1f))
            }
        }
    }
}

private fun Quick.icon(): ImageVector = when (this) {
    Quick.SendFile -> Icons.Filled.UploadFile
    Quick.SendClipboard -> Icons.Filled.ContentPaste
    Quick.Screen -> Icons.AutoMirrored.Filled.ScreenShare
    Quick.Camera -> Icons.Filled.Cameraswitch
    Quick.PhoneAudio -> Icons.AutoMirrored.Filled.VolumeUp
    Quick.PcAudio -> Icons.Filled.Computer
    Quick.Touchpad -> Icons.Filled.TouchApp
    Quick.Pair -> Icons.Filled.QrCodeScanner
}

/** One quick action: accent container while its share is live, dimmed and untappable when disabled. */
@Composable
private fun QuickCard(tile: QuickTile, modifier: Modifier, onClick: () -> Unit) {
    val colors = if (tile.on) {
        CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.primaryContainer, contentColor = MaterialTheme.colorScheme.onPrimaryContainer)
    } else {
        CardDefaults.cardColors(containerColor = cardColor(), contentColor = MaterialTheme.colorScheme.onSurface)
    }
    Card(
        onClick = onClick,
        shape = RoundedCornerShape(Radius.card),
        colors = colors,
        modifier = modifier.animateContentSize().alpha(if (tile.enabled) 1f else 0.5f)
            .semantics { if (!tile.enabled) stateDescription = "Unavailable" },
    ) {
        Column(Modifier.fillMaxWidth().padding(Spacing.lg), verticalArrangement = Arrangement.spacedBy(Spacing.sm)) {
            if (tile.on) Icon(tile.quick.icon(), null, Modifier.size(24.dp))
            else IconBadge(tile.quick.icon(), tintColor(if (tile.quick == Quick.Pair) RelayTint.Muted else RelayTint.Primary))
            Text(tile.label, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (tile.on) Text("On", style = MaterialTheme.typography.bodySmall, color = LocalContentColor.current.copy(alpha = 0.7f))
        }
    }
}

/** A live session the user can end from here. */
@Composable
private fun SessionCard(title: String, body: String, modifier: Modifier, onStop: () -> Unit) {
    Row(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Radius.banner))
            .background(MaterialTheme.colorScheme.primaryContainer)
            .animateContentSize()
            .padding(start = Spacing.lg, end = Spacing.sm, top = Spacing.md, bottom = Spacing.md),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, color = MaterialTheme.colorScheme.onPrimaryContainer, style = MaterialTheme.typography.titleSmall)
            Text(body, color = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.8f), style = MaterialTheme.typography.bodySmall)
        }
        Button(onClick = onStop, modifier = Modifier.semantics { contentDescription = "Stop. $title" }) { Text("Stop") }
    }
}

/** Screen and camera shares start on the PC; tapping them on the phone says where. */
private fun startsOnPc(ctx: Context, what: String) {
    Toast.makeText(ctx, startsOnPcText(what), Toast.LENGTH_LONG).show()
}

/**
 * "Remove and pair again": unpairs only the PCs found reset at this moment (the user asked for it;
 * plain Pair never removes anything) and drops their "Always allow" for the camera.
 */
private fun removeForgottenPcs(ctx: Context) {
    val node = Bridge.node ?: return
    runCatching {
        val gone = node.peers().filter { node.peerChanged(it.id) }
        if (gone.isEmpty()) return
        // A removal that could not be saved keeps the PC paired: say so, and go on with the rest.
        gone.forEach { p ->
            runCatching { node.tryUnpair(p.id) }.exceptionOrNull()?.let {
                notifyInfo(ctx, "Couldn't remove ${p.name}", "It is still paired. Try again.")
            }
        }
        val prefs = Prefs(ctx)
        prefs.dropAllowancesExcept(node.peers().map { it.id })
        Bridge.linked.value = node.peers().firstOrNull()?.name
        PbService.refresh(ctx)
    }
}

/** A system settings page; some OEM builds lack one, which must not crash Home. */
internal fun openSettings(ctx: Context, intent: Intent) {
    runCatching { ctx.startActivity(intent) }
        .onFailure { Toast.makeText(ctx, "This phone has no such settings page. Open Settings and find $APP_NAME there", Toast.LENGTH_LONG).show() }
}

internal fun openAppDetails(ctx: Context) {
    openSettings(ctx, Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).setData(Uri.parse("package:${ctx.packageName}")))
}

/** Names of the PCs flagged as changed right now, for the confirmation. */
private fun forgottenPcNames(): List<String> {
    val node = Bridge.node ?: return emptyList()
    return runCatching { node.peers().filter { node.peerChanged(it.id) }.map { it.name } }.getOrDefault(emptyList())
}

/** Red only when the PC no longer recognises this phone; optional permissions use the neutral warning surface. */
@Composable
private fun needsContainer(needs: List<Need>) =
    if (Need.PcForgotPhone in needs) MaterialTheme.colorScheme.errorContainer else MaterialTheme.colorScheme.secondaryContainer

@Composable
private fun needsContent(needs: List<Need>) =
    if (Need.PcForgotPhone in needs) MaterialTheme.colorScheme.onErrorContainer else MaterialTheme.colorScheme.onSecondaryContainer
