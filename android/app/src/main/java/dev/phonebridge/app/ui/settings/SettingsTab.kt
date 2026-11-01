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
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import dev.phonebridge.app.APP_NAME
import dev.phonebridge.app.Bridge
import dev.phonebridge.app.CameraShare
import dev.phonebridge.app.ClipImages
import dev.phonebridge.app.ControlService
import dev.phonebridge.app.Group
import dev.phonebridge.app.NotifyListener
import dev.phonebridge.app.PbService
import dev.phonebridge.app.Prefs
import dev.phonebridge.app.Radius
import dev.phonebridge.app.RelayDialog
import dev.phonebridge.app.ScanActivity
import dev.phonebridge.app.ScreenShare
import dev.phonebridge.app.ScreenshotWatcher
import dev.phonebridge.app.SettingsNav
import dev.phonebridge.app.groupTitle
import dev.phonebridge.app.hasNotifyAccess
import dev.phonebridge.app.isBatteryUnrestricted
import dev.phonebridge.app.pageSummary
import dev.phonebridge.app.pruneAllowed
import dev.phonebridge.app.searchSettings
import dev.phonebridge.app.setBridgeEnabled
import dev.phonebridge.app.treeLabel
import dev.phonebridge.app.ui.LocalResumeCount
import dev.phonebridge.app.ui.ThemeChoice
import dev.phonebridge.app.ui.components.SectionCard

/**
 * "Let my PC control this phone": on = the PC takes over without asking;
 * off = each request asks on the phone, and turning it off during a
 * session ends control. Shows who is in control, with a Stop button.
 */
@Composable
private fun ControlFromPc(modifier: Modifier = Modifier) {
    val ctx = LocalContext.current
    val resumed = LocalResumeCount.current
    val serviceOn = remember(resumed) { ControlService.instance != null }
    val serviceStuck = remember(resumed) { !serviceOn && ControlService.enabledInSettings(ctx) }
    val allow = Bridge.allowControl.value
    val controller = Bridge.controlledBy.value
    Column(modifier.fillMaxWidth()) {
        if (controller != null) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 4.dp)
                    .clip(RoundedCornerShape(Radius.banner))
                    .background(MaterialTheme.colorScheme.primaryContainer)
                    .padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    "$controller is controlling this phone",
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                    style = MaterialTheme.typography.titleSmall,
                    modifier = Modifier.weight(1f),
                )
                Button(onClick = { Bridge.node?.stopControl() }) { Text("Stop") }
            }
        }
        ListItem(
            headlineContent = { Text("Let my PC control this phone") },
            supportingContent = {
                Text(
                    when {
                        serviceStuck -> "Android stopped “$APP_NAME control”. Tap, then turn it off and on again"
                        !serviceOn -> "Needs \u201c$APP_NAME control\u201d in Accessibility settings. Tap to turn it on"
                        allow -> "On. Ctrl+Alt+P on your PC uses its mouse and keyboard here, and again to return"
                        else -> "Off. When your PC asks, this phone asks you first"
                    },
                )
            },
            trailingContent = {
                Switch(allow, { on ->
                    Bridge.setAllowControl(ctx, on)
                    if (on && !serviceOn) ctx.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
                })
            },
            modifier = Modifier.clickable(enabled = !serviceOn) { ctx.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) },
        )
    }
}

/**
 * "Screen to PC": who sees this screen right now (with Stop), and whether
 * a PC's request needs a yes first (Android's own prompt always shows).
 */
@Composable
private fun ScreenToPc(modifier: Modifier = Modifier) {
    val ctx = LocalContext.current
    val prefs = remember { Prefs(ctx) }
    var always by remember { mutableStateOf(prefs.allowScreen) }
    val viewer = ScreenShare.sharingWith.value
    Column(modifier.fillMaxWidth()) {
        if (viewer != null) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 4.dp)
                    .clip(RoundedCornerShape(Radius.banner))
                    .background(MaterialTheme.colorScheme.primaryContainer)
                    .padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    "$viewer can see this screen",
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                    style = MaterialTheme.typography.titleSmall,
                    modifier = Modifier.weight(1f),
                )
                Button(onClick = { ScreenShare.stop(userStopped = true) }) { Text("Stop") }
            }
        }
        var instant by remember { mutableStateOf(prefs.instantScreen) }
        ListItem(
            headlineContent = { Text("Instant screen") },
            supportingContent = {
                Text(
                    when {
                        !instant -> "Off. Android asks each time your PC shows this screen"
                        ScreenShare.ready.value -> "Ready. Android shows its cast icon while ready"
                        else -> "On. After the next share, later ones skip Android's prompt"
                    },
                )
            },
            trailingContent = {
                Switch(instant, { v ->
                    instant = v
                    prefs.instantScreen = v
                    if (!v) ScreenShare.disarm()
                })
            },
        )
        ListItem(
            headlineContent = { Text("Show my screen on my PC without asking") },
            supportingContent = {
                Text(
                    when {
                        !always -> "Off. This phone asks you first"
                        instant -> "On. With Instant screen on, your PC's shares start with no prompt at all"
                        else -> "On. Android still asks before each share"
                    },
                )
            },
            trailingContent = {
                Switch(always, { v ->
                    always = v
                    prefs.allowScreen = v
                })
            },
        )
    }
}

/** "Camera for my PC": who uses it now (with Stop), and the opt-in to skip the question. */
@Composable
private fun CameraForPc(modifier: Modifier = Modifier) {
    val ctx = LocalContext.current
    val resumed = LocalResumeCount.current
    val prefs = remember { Prefs(ctx) }
    var allowed by remember(resumed) { mutableStateOf(prefs.allowedCameraPcs) }
    val user = CameraShare.usedBy.value
    Column(modifier.fillMaxWidth()) {
        if (user != null) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 4.dp)
                    .clip(RoundedCornerShape(Radius.banner))
                    .background(MaterialTheme.colorScheme.primaryContainer)
                    .padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    "$user is using the camera",
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                    style = MaterialTheme.typography.titleSmall,
                    modifier = Modifier.weight(1f),
                )
                Button(onClick = { CameraShare.stop(userStopped = true) }) { Text("Stop") }
            }
        }
        ListItem(
            headlineContent = { Text("PCs always allowed to use the camera") },
            supportingContent = {
                Text(
                    if (allowed.isNotEmpty()) "${allowed.size} PC(s) skip the question. Android's camera indicator still shows"
                    else "None. This phone asks you first (choose Always allow in the question)",
                )
            },
            trailingContent = {
                // Turning off forgets every PC; a PC is added from the question.
                Switch(allowed.isNotEmpty(), { _ ->
                    allowed = emptySet()
                    prefs.allowedCameraPcs = emptySet()
                }, enabled = allowed.isNotEmpty())
            },
        )
    }
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
private fun NotificationsOnPc(modifier: Modifier = Modifier) {
    val ctx = LocalContext.current
    val resumed = LocalResumeCount.current
    val prefs = remember { Prefs(ctx) }
    var on by remember { mutableStateOf(prefs.notifyToPc) }
    val access = remember(resumed) { hasNotifyAccess(ctx) }
    val openAccess = { openNotifyAccess(ctx) }
    ListItem(
        headlineContent = { Text("Notifications on PC") },
        supportingContent = {
            Text(
                when {
                    !on -> "Off"
                    !access -> "Not allowed yet. Tap Grant, then turn on $APP_NAME"
                    else -> "Showing on your PC"
                },
            )
        },
        trailingContent = {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (on && !access) TextButton(onClick = openAccess) { Text("Grant") }
                Switch(on, { v ->
                    on = v
                    prefs.notifyToPc = v
                    Bridge.node?.setNotifyEnabled(v)
                    if (v) NotifyListener.instance?.seedAll()
                })
            }
        },
        modifier = modifier.clickable(enabled = on && !access) { openAccess() },
    )
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
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(nav.page?.let(::groupTitle) ?: "Settings") },
                // A top-level tab: the arrow shows only on a group's page.
                navigationIcon = {
                    if (nav.page != null) IconButton(onClick = { goBack() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                },
            )
        },
    ) { pad ->
        Crossfade(targetState = nav.page, animationSpec = tween(150), label = "settings page", modifier = Modifier.padding(pad)) { page ->
        LazyColumn(Modifier.fillMaxSize()) {
            if (page == null) {
                item {
                    OutlinedTextField(
                        value = nav.query,
                        onValueChange = { nav = nav.copy(query = it) },
                        singleLine = true,
                        placeholder = { Text("Search settings") },
                        leadingIcon = { Icon(Icons.Filled.Search, null) },
                        trailingIcon = {
                            if (nav.query.isNotEmpty()) IconButton(onClick = { nav = nav.copy(query = "") }) { Icon(Icons.Filled.Clear, "Clear search") }
                        },
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }
                if (nav.query.isNotBlank()) {
                    val hits = searchSettings(nav.query)
                    if (hits.isEmpty()) {
                        item { Text("No settings match", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(16.dp)) }
                    }
                    items(hits) { h ->
                        ListItem(
                            headlineContent = { Text(h.title) },
                            supportingContent = { Text("${groupTitle(h.group)}: ${h.subtitle}") },
                            modifier = Modifier.clickable { nav = nav.open(h.group) },
                        )
                    }
                } else {
                    items(Group.entries) { g ->
                        Card(
                            onClick = { nav = nav.open(g) },
                            shape = RoundedCornerShape(16.dp),
                            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
                            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
                            modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 12.dp),
                        ) {
                            ListItem(
                                headlineContent = { Text(groupTitle(g)) },
                                supportingContent = { Text(pageSummary(g)) },
                                colors = androidx.compose.material3.ListItemDefaults.colors(containerColor = androidx.compose.ui.graphics.Color.Transparent),
                            )
                        }
                    }
                }
            } else when (page) {
            Group.Connection -> item {
                GroupCard(null) {
                    peers.forEach { p ->
                        ListItem(
                            headlineContent = { Text(p.name) },
                            supportingContent = { Text("Fingerprint ${p.fingerprint.take(9)}") },
                            trailingContent = {
                                TextButton(onClick = { removing = p }) { Text("Remove", color = MaterialTheme.colorScheme.error) }
                            },
                        )
                    }
                    ListItem(
                        headlineContent = { Text("Pair a new PC", color = MaterialTheme.colorScheme.primary) },
                        modifier = Modifier.clickable { ctx.startActivity(Intent(ctx, ScanActivity::class.java)) },
                    )
                }
            }
            Group.Camera -> item {
                GroupCard(null) {
                    CameraForPc()
                }
            }
            Group.Screen -> item {
                GroupCard(null) {
                    ControlFromPc()
                    ScreenToPc()
                    val on = remember(resumed) { ControlService.instance != null }
                    val stuck = remember(resumed) { !on && ControlService.enabledInSettings(ctx) }
                    ListItem(
                        headlineContent = { Text("Accessibility permission") },
                        supportingContent = { Text("\u201c$APP_NAME control\u201d lets your PC tap and type here. Without it your PC cannot control this phone") },
                        trailingContent = { Text(if (on) "On" else if (stuck) "Stopped" else "Off", color = MaterialTheme.colorScheme.primary) },
                        modifier = Modifier.clickable { ctx.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) },
                    )
                }
            }
            Group.Notifications -> item {
                GroupCard(null) {
                    NotificationsOnPc()
                }
            }
            Group.Files -> item {
                GroupCard(null) {
                    ListItem(
                        headlineContent = { Text("Accept from my devices automatically") },
                        supportingContent = { Text("Only paired devices can send") },
                        trailingContent = { Switch(auto, { auto = it; prefs.autoAccept = it; Bridge.node?.setAutoAccept(it) }) },
                    )
                    ListItem(
                        headlineContent = { Text("Save files to") },
                        supportingContent = { Text(treeLabel(tree)) },
                        trailingContent = {
                            if (tree != null) {
                                TextButton(onClick = {
                                    tree?.let { runCatching { ctx.contentResolver.releasePersistableUriPermission(it, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION) } }
                                    prefs.saveTree = null
                                    tree = null
                                }) { Text("Reset") }
                            }
                        },
                        modifier = Modifier.clickable { pickFolder.launch(null) },
                    )
                }
            }
            Group.Settings -> item {
                GroupCard(null) {
                    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
                        listOf("System", "Light", "Dark").forEachIndexed { i, label ->
                            SegmentedButton(
                                selected = ThemeChoice.mode.intValue == i,
                                onClick = { ThemeChoice.mode.intValue = i; prefs.theme = i },
                                shape = SegmentedButtonDefaults.itemShape(i, 3),
                            ) { Text(label) }
                        }
                    }
                    ListItem(
                        headlineContent = { Text("Share clipboard") },
                        supportingContent = { Text("Share what you copy with your paired PC") },
                        trailingContent = { Switch(clipOn, { clipOn = it; prefs.clipboardSync = it; Bridge.node?.setClipboardSync(it) }) },
                    )
                    ListItem(
                        headlineContent = { Text("Passwords and codes") },
                        supportingContent = { Text("Off: text an app marks sensitive never leaves this phone. On: it is sent, kept out of clipboard history and hidden in notifications on the PC") },
                        trailingContent = { Switch(sens, { sens = it; prefs.sendSensitive = it }, enabled = clipOn) },
                    )
                    ListItem(
                        headlineContent = { Text("Share copied images") },
                        supportingContent = { Text("Pictures copied on your PC land on this phone's clipboard. Needs Share clipboard, and Share images on your PC") },
                        trailingContent = { Switch(images, { images = it; prefs.clipboardImages = it; Bridge.node?.setClipboardImages(it) }, enabled = clipOn) },
                    )
                    ListItem(
                        headlineContent = { Text("Send new screenshots to PC") },
                        supportingContent = { Text("Each screenshot you take is copied to your PC's clipboard. Only screenshots are sent, never other photos. Needs Share images on your PC") },
                        trailingContent = {
                            Switch(
                                shots,
                                {
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
                        },
                    )
                    Text(
                        "Android only lets apps read the clipboard while they are open. So copies made on this phone are shared when you open $APP_NAME or its Quick Settings panel (long-press the tile). Copies from your PC arrive any time.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                    )
                    ListItem(
                        headlineContent = { Text("$APP_NAME") },
                        supportingContent = { Text(if (Bridge.running.value) "On: your PC can reach this phone. Quick Settings tile turns it off" else "Off: your PC cannot reach this phone") },
                        trailingContent = { Switch(Bridge.running.value, { setBridgeEnabled(ctx, it) }) },
                    )
                    val unrestricted = remember(resumed) { mutableStateOf(isBatteryUnrestricted(ctx)) }
                    ListItem(
                        headlineContent = { Text("Keep running in background") },
                        supportingContent = {
                            Text(
                                if (unrestricted.value) "On. Android lets $APP_NAME come back by itself if the phone runs low on memory"
                                else "Recommended. Without it, Android may stop $APP_NAME when memory is low until you open it again",
                            )
                        },
                        trailingContent = { Text(if (unrestricted.value) "On" else "Off", color = MaterialTheme.colorScheme.primary) },
                        modifier = Modifier.clickable {
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
                    ListItem(
                        headlineContent = { Text("Hide the status notification") },
                        supportingContent = { Text("Android needs it while $APP_NAME runs in the background. You can turn it off for this app; $APP_NAME keeps working") },
                        modifier = Modifier.clickable {
                            ctx.startActivity(
                                Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
                                    .putExtra(Settings.EXTRA_APP_PACKAGE, ctx.packageName)
                                    .putExtra(Settings.EXTRA_CHANNEL_ID, "status"),
                            )
                        },
                    )
                }
            }
            }
            item { HorizontalDivider(Modifier.padding(vertical = 8.dp)) }
        }
        }
    }
}
@Composable
private fun Section(title: String) {
    Text(title, color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp))
}

/** A settings group: its header over a rounded card holding the group's rows. */
@Composable
private fun GroupCard(title: String?, content: @Composable () -> Unit) {
    if (title != null) Section(title)
    SectionCard { content() }
}
