package dev.phonebridge.app.ui.settings

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.PhotoCamera
import androidx.compose.material.icons.filled.ScreenShare
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import dev.phonebridge.app.APP_NAME
import dev.phonebridge.app.AppLog
import dev.phonebridge.app.Bridge
import dev.phonebridge.app.CameraShare
import dev.phonebridge.app.ClipImages
import dev.phonebridge.app.ControlService
import dev.phonebridge.app.Group
import dev.phonebridge.app.LogFields
import dev.phonebridge.app.NotifyListener
import dev.phonebridge.app.PbService
import dev.phonebridge.app.Prefs
import dev.phonebridge.app.Radius
import dev.phonebridge.app.RelayDialog
import dev.phonebridge.app.ScanActivity
import dev.phonebridge.app.ScreenShare
import dev.phonebridge.app.ScreenshotWatcher
import dev.phonebridge.app.SettingsNav
import dev.phonebridge.app.StatusInput
import dev.phonebridge.app.groupTitle
import dev.phonebridge.app.hasNotifyAccess
import dev.phonebridge.app.isBatteryUnrestricted
import dev.phonebridge.app.pruneAllowed
import dev.phonebridge.app.searchSettings
import dev.phonebridge.app.setBridgeEnabled
import dev.phonebridge.app.statusLine
import dev.phonebridge.app.treeLabel
import dev.phonebridge.app.ui.LocalResumeCount
import dev.phonebridge.app.ui.ThemeChoice
import dev.phonebridge.app.ui.Tokens
import dev.phonebridge.app.ui.isDark

/** A "who is using this now" strip with a Stop button, shown above a page's rows while it applies. */
@Composable
private fun LiveBanner(text: String, onStop: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(start = 12.dp, end = 12.dp, top = 12.dp, bottom = 4.dp)
            .clip(RoundedCornerShape(Radius.chip))
            .background(MaterialTheme.colorScheme.primaryContainer)
            .padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text,
            color = MaterialTheme.colorScheme.onPrimaryContainer,
            style = MaterialTheme.typography.titleSmall,
            modifier = Modifier.weight(1f),
        )
        Button(onClick = onStop, modifier = Modifier.heightIn(min = 48.dp)) { Text("Stop") }
    }
}

/**
 * "Let my PC control this phone": on = the PC takes over without asking;
 * off = each request asks on the phone, and turning it off during a
 * session ends control. Shows who is in control, with a Stop button.
 */
@Composable
private fun ControlFromPc() {
    val ctx = LocalContext.current
    val resumed = LocalResumeCount.current
    val serviceOn = remember(resumed) { ControlService.instance != null }
    val serviceStuck = remember(resumed) { !serviceOn && ControlService.enabledInSettings(ctx) }
    val allow = Bridge.allowControl.value
    val controller = Bridge.controlledBy.value
    if (controller != null) LiveBanner("$controller is controlling this phone") { Bridge.node?.stopControl() }
    SettingItem(
        title = "Let my PC control this phone",
        supporting = when {
            serviceStuck -> "Android stopped “$APP_NAME control”. Tap, then turn it off and on again"
            !serviceOn -> "Needs “$APP_NAME control” in Accessibility settings. Tap to turn it on"
            allow -> "On. Ctrl+Alt+P on your PC uses its mouse and keyboard here, and again to return"
            else -> "Off. When your PC asks, this phone asks you first"
        },
        checked = allow,
        onChecked = { on ->
            Bridge.setAllowControl(ctx, on)
            if (on && !serviceOn) ctx.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
        },
        onClick = if (!serviceOn) ({ ctx.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }) else null,
    )
}

/**
 * "Screen to PC": who sees this screen right now (with Stop), and whether
 * a PC's request needs a yes first (Android's own prompt always shows).
 */
@Composable
private fun ScreenToPc() {
    val ctx = LocalContext.current
    val prefs = remember { Prefs(ctx) }
    var always by remember { mutableStateOf(prefs.allowScreen) }
    val viewer = ScreenShare.sharingWith.value
    if (viewer != null) LiveBanner("$viewer can see this screen") { ScreenShare.stop(userStopped = true) }
    var instant by remember { mutableStateOf(prefs.instantScreen) }
    SettingItem(
        title = "Instant screen",
        supporting = when {
            !instant -> "Off. Android asks each time your PC shows this screen"
            ScreenShare.ready.value -> "Ready. Android shows its cast icon while ready"
            else -> "On. After the next share, later ones skip Android's prompt"
        },
        checked = instant,
        onChecked = { v ->
            instant = v
            prefs.instantScreen = v
            if (!v) ScreenShare.disarm()
        },
    )
    RowDivider()
    SettingItem(
        title = "Show my screen on my PC without asking",
        supporting = when {
            !always -> "Off. This phone asks you first"
            instant -> "On. With Instant screen on, your PC's shares start with no prompt at all"
            else -> "On. Android still asks before each share"
        },
        checked = always,
        onChecked = { v ->
            always = v
            prefs.allowScreen = v
        },
    )
}

/** "Camera for my PC": who uses it now (with Stop), and the opt-in to skip the question. */
@Composable
private fun CameraForPc() {
    val resumed = LocalResumeCount.current
    val ctx = LocalContext.current
    val prefs = remember { Prefs(ctx) }
    var allowed by remember(resumed) { mutableStateOf(prefs.allowedCameraPcs) }
    val user = CameraShare.usedBy.value
    if (user != null) LiveBanner("$user is using the camera") { CameraShare.stop(userStopped = true) }
    SettingItem(
        title = "PCs always allowed to use the camera",
        supporting = if (allowed.isNotEmpty()) "${allowed.size} PC(s) skip the question. Android's camera indicator still shows"
        else "None. This phone asks you first (choose Always allow in the question)",
        // Turning off forgets every PC; a PC is added from the question.
        checked = allowed.isNotEmpty(),
        onChecked = { _ ->
            allowed = emptySet()
            prefs.allowedCameraPcs = emptySet()
        },
        switchEnabled = allowed.isNotEmpty(),
    )
}

/**
 * "Notifications on PC": phone notifications appear in the PC's
 * notification centre. Needs Notification access, granted in Settings.
 */
fun openNotifyAccess(ctx: Context) {
    val i = if (Build.VERSION.SDK_INT >= 30) {
        Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS).putExtra(
            Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME,
            android.content.ComponentName(ctx, NotifyListener::class.java).flattenToString(),
        )
    } else {
        Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS")
    }
    runCatching { ctx.startActivity(i) }
        .onFailure { ctx.startActivity(Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS")) }
}

@Composable
private fun NotificationsOnPc() {
    val ctx = LocalContext.current
    val resumed = LocalResumeCount.current
    val prefs = remember { Prefs(ctx) }
    var on by remember { mutableStateOf(prefs.notifyToPc) }
    val access = remember(resumed) { hasNotifyAccess(ctx) }
    val openAccess = { openNotifyAccess(ctx) }
    SettingItem(
        title = "Notifications on PC",
        supporting = when {
            !on -> "Off"
            !access -> "Not allowed yet. Tap Grant, then turn on $APP_NAME"
            else -> "Showing on your PC"
        },
        checked = on,
        onChecked = { v ->
            on = v
            prefs.notifyToPc = v
            Bridge.node?.setNotifyEnabled(v)
            if (v) NotifyListener.instance?.seedAll()
        },
        onClick = if (on && !access) openAccess else null,
        trailing = if (on && !access) ({ TextButton(onClick = openAccess, modifier = Modifier.heightIn(min = 48.dp)) { Text("Grant") } }) else null,
    )
}

private val GroupIcon: Map<Group, ImageVector> = mapOf(
    Group.Connection to Icons.Filled.Link,
    Group.Camera to Icons.Filled.PhotoCamera,
    Group.Screen to Icons.Filled.ScreenShare,
    Group.Notifications to Icons.Filled.Notifications,
    Group.Files to Icons.Filled.Folder,
    Group.Settings to Icons.Filled.Tune,
)

/** Category tints, from the shared tokens so they follow light and dark. */
@Composable
private fun groupTint(g: Group): Color {
    val t = if (isDark()) Tokens.Dark else Tokens.Light
    return when (g) {
        Group.Connection -> t.primary
        Group.Camera -> t.accent
        Group.Screen -> t.success
        Group.Notifications -> t.warning
        Group.Files -> t.primary
        Group.Settings -> t.textMuted
    }
}

/**
 * The Settings tab: the group list with search, and one page per group. Back closes a page or
 * clears the search first; at the top level it is the nav host's (to Home). [onDone] leaves the
 * tab after removing a PC.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsTab(onDone: () -> Unit) {
    val ctx = LocalContext.current
    val resumed = LocalResumeCount.current
    val prefs = remember { Prefs(ctx) }
    var nav by remember { mutableStateOf(SettingsNav()) }
    val goBack = { nav.back()?.let { nav = it } }
    BackHandler(enabled = nav.back() != null) { goBack() }
    var clipOn by remember(resumed) { mutableStateOf(prefs.clipboardSync) }
    var sens by remember(resumed) { mutableStateOf(prefs.sendSensitive) }
    var images by remember(resumed) { mutableStateOf(prefs.clipboardImages) }
    var shots by remember(resumed) { mutableStateOf(prefs.shareScreenshots && ClipImages.hasImageAccess(ctx)) }
    // Android's own picture-access prompt: the user answers it, the switch follows the answer.
    val askPictures = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        shots = granted
        prefs.shareScreenshots = granted
        ScreenshotWatcher.sync(ctx)
    }
    var auto by remember(resumed) { mutableStateOf(prefs.autoAccept) }
    var tree by remember { mutableStateOf(prefs.saveTree) }
    val peers = remember(resumed, Bridge.linked.value, Bridge.nodeReady.value) { Bridge.node?.peers().orEmpty() }
    var removing by remember { mutableStateOf<uniffi.pb_ffi.PbPeer?>(null) }
    removing?.let { p ->
        RelayDialog(
            title = "Remove ${p.name}?",
            onDismissRequest = { removing = null },
            confirmText = "Remove",
            destructive = true,
            onConfirm = {
                removing = null
                Bridge.node?.unpair(p.id)
                Prefs(ctx).let { it.allowedCameraPcs = pruneAllowed(it.allowedCameraPcs, Bridge.node?.peers()?.map { q -> q.id }.orEmpty()) }
                Bridge.linked.value = Bridge.node?.peers()?.firstOrNull()?.name
                PbService.refresh(ctx)
                onDone()
            },
            dismissText = "Cancel",
        ) {
            Text("It will no longer be able to connect. You can pair again later by scanning its QR code.", style = MaterialTheme.typography.bodyMedium)
        }
    }
    val pickFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri: Uri? ->
        if (uri != null) {
            ctx.contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
            prefs.saveTree?.let { old -> runCatching { ctx.contentResolver.releasePersistableUriPermission(old, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION) } }
            prefs.saveTree = uri
            tree = uri
        }
    }
    // Live status lines; re-read on resume and whenever a page is left, so they follow what was changed there.
    val status = remember(resumed, nav.page, peers, tree, Bridge.allowControl.value) {
        StatusInput(
            pairedNames = peers.map { it.name },
            cameraAllowedCount = prefs.allowedCameraPcs.size,
            controlAllowed = Bridge.allowControl.value,
            controlServiceOn = ControlService.instance != null,
            notifyOn = prefs.notifyToPc,
            notifyAccess = hasNotifyAccess(ctx),
            saveLabel = treeLabel(tree),
            themeName = listOf("System", "Light", "Dark").getOrElse(ThemeChoice.mode.intValue) { "System" },
            keepLogs = prefs.keepLogs,
            logSize = LogFields.mb(AppLog.sizeBytes()),
        )
    }
    val fadeMs = if (animationsEnabled(ctx)) 160 else 0
    Scaffold(
        containerColor = MaterialTheme.colorScheme.surface,
        topBar = {
            TopAppBar(
                title = { Text(nav.page?.let(::groupTitle) ?: "Settings", style = MaterialTheme.typography.titleLarge) },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surface),
                // A top-level tab: the arrow shows only on a group's page.
                navigationIcon = {
                    if (nav.page != null) IconButton(onClick = { goBack() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                },
            )
        },
    ) { pad ->
        Crossfade(targetState = nav.page, animationSpec = tween(fadeMs), label = "settings page", modifier = Modifier.padding(pad)) { page ->
            LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 24.dp)) {
                if (page == null) {
                    item { PillSearch(nav.query, { nav = nav.copy(query = it) }) }
                    if (nav.query.isNotBlank()) {
                        val hits = searchSettings(nav.query)
                        if (hits.isEmpty()) {
                            item { Text("No settings match", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(16.dp)) }
                        } else {
                            item {
                                SettingsCard {
                                    hits.forEachIndexed { i, h ->
                                        if (i > 0) RowDivider(inset = true)
                                        CategoryRow(GroupIcon.getValue(h.group), groupTint(h.group), h.title, "${groupTitle(h.group)}: ${h.subtitle}") { nav = nav.open(h.group) }
                                    }
                                }
                            }
                        }
                    } else {
                        for ((header, groups) in listOf(
                            "Your PC" to listOf(Group.Connection),
                            "Share with your PC" to listOf(Group.Camera, Group.Screen, Group.Notifications, Group.Files),
                            "This phone" to listOf(Group.Settings),
                        )) {
                            item { SettingsHeader(header) }
                            item {
                                SettingsCard {
                                    groups.forEachIndexed { i, g ->
                                        if (i > 0) RowDivider(inset = true)
                                        CategoryRow(GroupIcon.getValue(g), groupTint(g), groupTitle(g), statusLine(g, status)) { nav = nav.open(g) }
                                    }
                                }
                            }
                        }
                    }
                } else when (page) {
                    Group.Connection -> {
                        item { SettingsHeader("Paired PCs") }
                        item {
                            SettingsCard {
                                if (peers.isEmpty()) SettingItem("No PC paired yet", "Scan the QR code shown on your PC")
                                peers.forEachIndexed { i, p ->
                                    if (i > 0) RowDivider()
                                    SettingItem(
                                        title = p.name,
                                        supporting = "Fingerprint ${p.fingerprint.take(9)}",
                                        trailing = {
                                            TextButton(onClick = { removing = p }, modifier = Modifier.heightIn(min = 48.dp)) {
                                                Text("Remove", color = MaterialTheme.colorScheme.error)
                                            }
                                        },
                                    )
                                }
                            }
                        }
                        item {
                            SettingsCard {
                                SettingItem(
                                    title = "Pair a new PC",
                                    supporting = "Scan the QR code shown on your PC",
                                    titleColor = MaterialTheme.colorScheme.primary,
                                    trailing = { Icon(Icons.Filled.Add, null, tint = MaterialTheme.colorScheme.primary) },
                                    onClick = { ctx.startActivity(Intent(ctx, ScanActivity::class.java)) },
                                )
                            }
                        }
                    }
                    Group.Camera -> item { SettingsCard { CameraForPc() } }
                    Group.Screen -> {
                        item { SettingsHeader("Control from your PC") }
                        item { SettingsCard { ControlFromPc() } }
                        item { SettingsHeader("Showing this screen") }
                        item { SettingsCard { ScreenToPc() } }
                        item {
                            SettingsCard {
                                val on = remember(resumed) { ControlService.instance != null }
                                val stuck = remember(resumed) { !on && ControlService.enabledInSettings(ctx) }
                                SettingItem(
                                    title = "Accessibility permission",
                                    supporting = "“$APP_NAME control” lets your PC tap and type here. Without it your PC cannot control this phone",
                                    trailing = { Text(if (on) "On" else if (stuck) "Stopped" else "Off", color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.labelLarge) },
                                    onClick = { ctx.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) },
                                )
                            }
                        }
                    }
                    Group.Notifications -> item { SettingsCard { NotificationsOnPc() } }
                    Group.Files -> {
                        item { SettingsHeader("Receiving") }
                        item {
                            SettingsCard {
                                SettingItem(
                                    title = "Accept from my devices automatically",
                                    supporting = "Only paired devices can send",
                                    checked = auto,
                                    onChecked = { auto = it; prefs.autoAccept = it; Bridge.node?.setAutoAccept(it) },
                                )
                                RowDivider()
                                SettingItem(
                                    title = "Save files to",
                                    supporting = treeLabel(tree),
                                    trailing = if (tree != null) ({
                                        TextButton(
                                            onClick = {
                                                tree?.let { runCatching { ctx.contentResolver.releasePersistableUriPermission(it, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION) } }
                                                prefs.saveTree = null
                                                tree = null
                                            },
                                            modifier = Modifier.heightIn(min = 48.dp),
                                        ) { Text("Reset") }
                                    }) else null,
                                    onClick = { pickFolder.launch(null) },
                                )
                            }
                        }
                    }
                    Group.Settings -> {
                        item { SettingsHeader("Appearance") }
                        item {
                            SettingsCard {
                                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(16.dp)) {
                                    listOf("System", "Light", "Dark").forEachIndexed { i, label ->
                                        SegmentedButton(
                                            selected = ThemeChoice.mode.intValue == i,
                                            onClick = { ThemeChoice.mode.intValue = i; prefs.theme = i },
                                            shape = SegmentedButtonDefaults.itemShape(i, 3),
                                            modifier = Modifier.heightIn(min = 48.dp),
                                        ) { Text(label) }
                                    }
                                }
                            }
                        }
                        item { SettingsHeader("Clipboard") }
                        item {
                            SettingsCard {
                                SettingItem(
                                    title = "Share clipboard",
                                    supporting = "Share what you copy with your paired PC",
                                    checked = clipOn,
                                    onChecked = { clipOn = it; prefs.clipboardSync = it; Bridge.node?.setClipboardSync(it) },
                                )
                                RowDivider()
                                SettingItem(
                                    title = "Passwords and codes",
                                    supporting = "Off: text an app marks sensitive never leaves this phone. On: it is sent, kept out of clipboard history and hidden in notifications on the PC",
                                    checked = sens,
                                    onChecked = { sens = it; prefs.sendSensitive = it },
                                    switchEnabled = clipOn,
                                )
                                RowDivider()
                                SettingItem(
                                    title = "Share copied images",
                                    supporting = "Pictures copied on your PC land on this phone's clipboard. Needs Share clipboard, and Share images on your PC",
                                    checked = images,
                                    onChecked = { images = it; prefs.clipboardImages = it; Bridge.node?.setClipboardImages(it) },
                                    switchEnabled = clipOn,
                                )
                                RowDivider()
                                SettingItem(
                                    title = "Send new screenshots to PC",
                                    supporting = "Each screenshot you take is copied to your PC's clipboard. Only screenshots are sent, never other photos. Needs Share images on your PC",
                                    checked = shots,
                                    onChecked = {
                                        if (!it) {
                                            shots = false
                                            prefs.shareScreenshots = false
                                            ScreenshotWatcher.sync(ctx)
                                        } else if (ClipImages.hasImageAccess(ctx)) {
                                            shots = true
                                            prefs.shareScreenshots = true
                                            ScreenshotWatcher.sync(ctx)
                                        } else {
                                            askPictures.launch(ClipImages.imagePermission)
                                        }
                                    },
                                )
                            }
                        }
                        item {
                            Text(
                                "Android only lets apps read the clipboard while they are open. So copies made on this phone are shared when you open $APP_NAME or its Quick Settings panel (long-press the tile). Copies from your PC arrive any time.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 32.dp, vertical = 4.dp),
                            )
                        }
                        item { SettingsHeader("Background") }
                        item {
                            SettingsCard {
                                SettingItem(
                                    title = APP_NAME,
                                    supporting = if (Bridge.running.value) "On: your PC can reach this phone. Quick Settings tile turns it off" else "Off: your PC cannot reach this phone",
                                    checked = Bridge.running.value,
                                    onChecked = { setBridgeEnabled(ctx, it) },
                                )
                                RowDivider()
                                val unrestricted = remember(resumed) { mutableStateOf(isBatteryUnrestricted(ctx)) }
                                SettingItem(
                                    title = "Keep running in background",
                                    supporting = if (unrestricted.value) "On. Android lets $APP_NAME come back by itself if the phone runs low on memory"
                                    else "Recommended. Without it, Android may stop $APP_NAME when memory is low until you open it again",
                                    trailing = { Text(if (unrestricted.value) "On" else "Off", color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.labelLarge) },
                                    onClick = {
                                        if (!unrestricted.value) {
                                            ctx.startActivity(
                                                Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)
                                                    .setData(Uri.parse("package:${ctx.packageName}")),
                                            )
                                        } else {
                                            ctx.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
                                        }
                                    },
                                )
                                RowDivider()
                                SettingItem(
                                    title = "Hide the status notification",
                                    supporting = "Android needs it while $APP_NAME runs in the background. You can turn it off for this app; $APP_NAME keeps working",
                                    onClick = {
                                        ctx.startActivity(
                                            Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
                                                .putExtra(Settings.EXTRA_APP_PACKAGE, ctx.packageName)
                                                .putExtra(Settings.EXTRA_CHANNEL_ID, "status"),
                                        )
                                    },
                                )
                            }
                        }
                        item { SettingsHeader("Logs") }
                        item { SettingsCard { LogRows() } }
                    }
                }
            }
        }
    }
}

/** Keep logs, Log size, Export logs and Clear all logs: the day-by-day log on this phone. */
@Composable
private fun LogRows() {
    val ctx = LocalContext.current
    val prefs = remember { Prefs(ctx) }
    var keep by remember { mutableStateOf(prefs.keepLogs) }
    var size by remember { mutableStateOf(AppLog.sizeBytes()) }
    var days by remember { mutableStateOf(AppLog.dayCount()) }
    var confirming by remember { mutableStateOf(false) }
    var note by remember { mutableStateOf<String?>(null) }
    fun refresh() {
        size = AppLog.sizeBytes()
        days = AppLog.dayCount()
    }
    if (confirming) {
        RelayDialog(
            title = "Clear all logs?",
            onDismissRequest = { confirming = false },
            confirmText = "Clear",
            destructive = true,
            onConfirm = {
                confirming = false
                note = if (AppLog.clearAll()) null else "Could not clear the logs"
                refresh()
            },
            dismissText = "Cancel",
        ) {
            Text("This deletes ${LogFields.mb(size)} of logs from this phone. Nothing else is affected.", style = MaterialTheme.typography.bodyMedium)
        }
    }
    SettingItem(
        title = "Keep logs",
        supporting = "Technical notes about connections and transfers, to find problems. Never clipboard text, notifications or file contents",
        checked = keep,
        onChecked = { keep = it; prefs.keepLogs = it; AppLog.setEnabled(it); refresh() },
    )
    RowDivider()
    SettingItem(
        title = "Log size",
        supporting = note ?: "${LogFields.mb(size)} in $days ${if (days == 1) "day" else "days"}",
        onClick = { refresh() },
    )
    RowDivider()
    SettingItem(
        title = "Export logs",
        supporting = "Share a zip of the daily logs",
        onClick = {
            val i = runCatching { AppLog.exportIntent(ctx) }.getOrNull()
            if (i == null) note = "No logs to export" else runCatching { ctx.startActivity(i) }
        },
    )
    RowDivider()
    SettingItem(
        title = "Clear all logs",
        supporting = "Delete every daily log on this phone",
        danger = true,
        onClick = { refresh(); confirming = true },
    )
}
