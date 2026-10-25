package dev.phonebridge.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color as AColor
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch

private const val TAG = "PhoneBridge"

/**
 * UI over the [PbService]-owned node. Screens follow design v2: Home (link
 * status, Share clipboard, recent), Settings, and the pairing code check.
 *
 * Test hooks (debug use over adb): intent extras `pair_code`, `confirm` ("yes"/"no"),
 * `send_text`. They drive exactly the same code paths as the UI.
 */
class MainActivity : ComponentActivity() {
    private val sas = Bridge.sas
    private val message = Bridge.pairMessage
    private lateinit var prefs: Prefs
    /** Bumped on every resume: rows showing system settings re-read them. */
    private val resumed = mutableIntStateOf(0)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        prefs = Prefs(this)
        ThemeChoice.mode.intValue = prefs.theme
        Bridge.allowControl.value = prefs.allowControl
        if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        honourUserStop(this)
        if (prefs.enabled) startForegroundService(Intent(this, PbService::class.java))
        setContent {
            PbTheme {
                val dark = isDark()
                LaunchedEffect(dark) {
                    val bars = if (dark) SystemBarStyle.dark(AColor.TRANSPARENT) else SystemBarStyle.light(AColor.TRANSPARENT, AColor.TRANSPARENT)
                    enableEdgeToEdge(statusBarStyle = bars, navigationBarStyle = bars)
                }
                Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) { App() }
            }
        }
        if (savedInstanceState == null) handleIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        resumed.intValue++
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        // Share clipboard, phone -> PC: readable only while we have focus.
        if (hasFocus) shareClipboardIfChanged(this, prefs)
    }

    private fun handleIntent(intent: Intent?) {
        intent ?: return
        // Test hooks: MainActivity is exported, so in a release build any app
        // could use them to pair this phone with its own PC. Debug builds only.
        if (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE == 0) return
        if (!intent.hasExtra("pair_code") && !intent.hasExtra("confirm") && !intent.hasExtra("send_text")) return
        Bridge.scope.launch {
            val node = Bridge.awaitNode() ?: return@launch
            intent.getStringExtra("pair_code")?.let { Bridge.join(it) }
            intent.getStringExtra("confirm")?.let { Bridge.confirm(applicationContext, it == "yes") }
            intent.getStringExtra("send_text")?.let { t ->
                val peer = node.peers().firstOrNull() ?: return@let
                runCatching { node.sendText(peer.id, t, false) }.onSuccess { Log.i(TAG, "SENT ${t.length} chars to ${peer.name}") }
            }
        }
    }

    @Composable
    private fun App() {
        var screen by remember { mutableStateOf("home") }
        val scope = rememberCoroutineScope()
        Crossfade(targetState = screen, animationSpec = tween(150), label = "screen") { s ->
            when (s) {
                "settings" -> {
                    BackHandler { screen = "home" }
                    SettingsScreen(onBack = { screen = "home" })
                }
                else -> HomeScreen(onSettings = { screen = "settings" })
            }
        }
        sas.value?.let { code ->
            val amber = if (isDark()) AmberDark else Amber
            AlertDialog(
                onDismissRequest = {},
                title = { Text("Does the PC show this code?") },
                text = {
                    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            listOf(code.take(3), code.drop(3)).forEach {
                                Text(
                                    it,
                                    fontSize = 40.sp,
                                    fontWeight = FontWeight.Medium,
                                    color = amber,
                                    modifier = Modifier
                                        .clip(RoundedCornerShape(14.dp))
                                        .background(amber.copy(alpha = 0.16f))
                                        .padding(horizontal = 12.dp, vertical = 2.dp),
                                )
                            }
                        }
                        Text("If the codes are different, choose No. Someone may be trying to intercept the connection.", style = MaterialTheme.typography.bodyMedium)
                    }
                },
                confirmButton = { Button(onClick = { scope.launch { Bridge.confirm(applicationContext, true) } }) { Text("Yes, pair") } },
                dismissButton = { OutlinedButton(onClick = { scope.launch { Bridge.confirm(applicationContext, false) } }) { Text("No") } },
            )
        }
        message.value?.let { m ->
            AlertDialog(onDismissRequest = { message.value = null }, text = { Text(m) }, confirmButton = { TextButton(onClick = { message.value = null }) { Text("OK") } })
        }
    }

    /**
     * "Let my PC control this phone": on = the PC takes over without asking;
     * off = each request asks on the phone, and turning it off during a
     * session ends control. Shows who is in control, with a Stop button.
     */
    @Composable
    private fun ControlFromPc(modifier: Modifier = Modifier) {
        val serviceOn = remember(resumed.intValue) { ControlService.instance != null }
        val serviceStuck = remember(resumed.intValue) { !serviceOn && ControlService.enabledInSettings(this@MainActivity) }
        val allow = Bridge.allowControl.value
        val controller = Bridge.controlledBy.value
        Column(modifier.fillMaxWidth()) {
            if (controller != null) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp)
                        .clip(RoundedCornerShape(20.dp))
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
                        Bridge.setAllowControl(this@MainActivity, on)
                        if (on && !serviceOn) startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
                    })
                },
                modifier = Modifier.clickable(enabled = !serviceOn) { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) },
            )
        }
    }

    /**
     * "Screen to PC": who sees this screen right now (with Stop), and whether
     * a PC's request needs a yes first (Android's own prompt always shows).
     */
    @Composable
    private fun ScreenToPc(modifier: Modifier = Modifier) {
        val prefs = remember { Prefs(this@MainActivity) }
        var always by remember { mutableStateOf(prefs.allowScreen) }
        val viewer = ScreenShare.sharingWith.value
        Column(modifier.fillMaxWidth()) {
            if (viewer != null) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp)
                        .clip(RoundedCornerShape(20.dp))
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
        val prefs = remember { Prefs(this@MainActivity) }
        var allowed by remember(resumed.intValue) { mutableStateOf(prefs.allowedCameraPcs) }
        val user = CameraShare.usedBy.value
        Column(modifier.fillMaxWidth()) {
            if (user != null) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp)
                        .clip(RoundedCornerShape(20.dp))
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
    @Composable
    private fun NotificationsOnPc(modifier: Modifier = Modifier) {
        val prefs = remember { Prefs(this@MainActivity) }
        var on by remember { mutableStateOf(prefs.notifyToPc) }
        val access = remember(resumed.intValue) { hasNotifyAccess(this@MainActivity) }
        val openAccess = {
            val i = if (Build.VERSION.SDK_INT >= 30) {
                Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS).putExtra(
                    Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME,
                    android.content.ComponentName(this@MainActivity, NotifyListener::class.java).flattenToString(),
                )
            } else {
                Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS")
            }
            runCatching { startActivity(i) }
                .onFailure { startActivity(Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS")) }
            Unit
        }
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

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    private fun HomeScreen(onSettings: () -> Unit) {
        var clipOn by remember(resumed.intValue) { mutableStateOf(prefs.clipboardSync) }
        val pc = Bridge.linked.value
        val running = Bridge.running.value
        val notificationsOn = remember(resumed.intValue) { getSystemService(android.app.NotificationManager::class.java).areNotificationsEnabled() }
        Scaffold(
            topBar = {
                TopAppBar(
                    title = { Text("$APP_NAME") },
                    actions = { IconButton(onClick = onSettings) { Icon(Icons.Filled.Settings, "Settings") } },
                )
            },
        ) { pad ->
            LazyColumn(Modifier.padding(pad).padding(horizontal = 16.dp)) {
                item {
                    Column(
                        Modifier.fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(MaterialTheme.colorScheme.surfaceContainer).padding(20.dp),
                        verticalArrangement = Arrangement.spacedBy(14.dp),
                    ) {
                        LinkIndicator(Build.MODEL ?: "Phone", pc, running)
                        Text(
                            when {
                                !running -> "$APP_NAME is off. Your PC cannot reach this phone."
                                !Bridge.nodeReady.value -> "Starting\u2026"
                                pc != null -> "Paired with $pc. Anything you share to $APP_NAME goes to your PC."
                                else -> "Not paired yet. On your PC, open $APP_NAME and choose Pair new device, then tap Pair below."
                            },
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (!running) {
                            Button(onClick = { setBridgeEnabled(this@MainActivity, true) }) { Text("Turn on") }
                        } else if (!Bridge.nodeReady.value) {
                            // Starting: nothing to tap yet.
                        } else if (pc == null) {
                            Button(onClick = { startActivity(Intent(this@MainActivity, ScanActivity::class.java)) }) { Text("Pair: scan QR from PC") }
                        } else {
                            Button(onClick = { startActivity(Intent(this@MainActivity, TouchpadActivity::class.java)) }) { Text("Use as touchpad and keyboard") }
                        }
                    }
                }
                Bridge.controlledBy.value?.let { controller ->
                    item {
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .padding(top = 8.dp)
                                .clip(RoundedCornerShape(20.dp))
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
                }
                item {
                    val tiles = homeTiles(pc != null && running, CameraShare.usedBy.value != null, ScreenShare.sharingWith.value != null, clipOn)
                    Column(Modifier.padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        tiles.chunked(2).forEach { row ->
                            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                row.forEach { tile ->
                                    // Camera and Screen start from the PC: tapping while live stops the share (same as the old Stop buttons).
                                    // Send file has no action on the phone (files are shared from other apps), so it is a status tile.
                                    val action: (() -> Unit)? = when (tile.group) {
                                        Group.Camera -> if (tile.on) ({ CameraShare.stop(userStopped = true) }) else null
                                        Group.Screen -> if (tile.on) ({ ScreenShare.stop(userStopped = true) }) else null
                                        Group.Settings -> ({
                                            clipOn = !clipOn
                                            prefs.clipboardSync = clipOn
                                            Bridge.node?.setClipboardSync(clipOn)
                                        })
                                        else -> null
                                    }
                                    HomeTileCard(tile, action, Modifier.weight(1f))
                                }
                            }
                        }
                    }
                }
                if (!notificationsOn) {
                    item {
                        ListItem(
                            headlineContent = { Text("Notifications are off") },
                            supportingContent = { Text("You will not see files, links or clips arriving from your PC. Tap to turn them on") },
                            modifier = Modifier.clickable {
                                startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, packageName))
                            },
                        )
                    }
                }
                item { Text("Recent", color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(start = 16.dp, top = 8.dp)) }
                if (Bridge.recent.isEmpty()) {
                    item { Text("Nothing yet. Share a photo or file to $APP_NAME from any app.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(16.dp)) }
                }
                items(Bridge.recent) { r ->
                    ListItem(
                        headlineContent = { Text(r.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        supportingContent = { Text(r.subtitle) },
                        leadingContent = { KindBadge(r.kind) },
                        modifier = Modifier.clickable(enabled = r.uri != null) {
                            r.uri?.let {
                                // No app for this file type, or the file is gone.
                                runCatching { startActivity(Intent(Intent.ACTION_VIEW).setData(it).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)) }
                                    .onFailure { message.value = "No app can open ${r.title}" }
                            }
                        },
                    )
                }
            }
        }
    }

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    private fun SettingsScreen(onBack: () -> Unit) {
        var clipOn by remember(resumed.intValue) { mutableStateOf(prefs.clipboardSync) }
        var sens by remember(resumed.intValue) { mutableStateOf(prefs.sendSensitive) }
        var auto by remember(resumed.intValue) { mutableStateOf(prefs.autoAccept) }
        var tree by remember { mutableStateOf(prefs.saveTree) }
        val peers = remember(resumed.intValue, Bridge.linked.value, Bridge.nodeReady.value) { Bridge.node?.peers().orEmpty() }
        var removing by remember { mutableStateOf<uniffi.pb_ffi.PbPeer?>(null) }
        removing?.let { p ->
            AlertDialog(
                onDismissRequest = { removing = null },
                title = { Text("Remove ${p.name}?") },
                text = { Text("It will no longer be able to connect. You can pair again later by scanning its QR code.") },
                confirmButton = {
                    TextButton(onClick = {
                        removing = null
                        Bridge.node?.unpair(p.id)
                        Prefs(this@MainActivity).let { it.allowedCameraPcs = pruneAllowed(it.allowedCameraPcs, Bridge.node?.peers()?.map { q -> q.id }.orEmpty()) }
                        Bridge.linked.value = Bridge.node?.peers()?.firstOrNull()?.name
                        PbService.refresh(this@MainActivity)
                        onBack()
                    }) { Text("Remove", color = MaterialTheme.colorScheme.error) }
                },
                dismissButton = { TextButton(onClick = { removing = null }) { Text("Cancel") } },
            )
        }
        val pickFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri: Uri? ->
            if (uri != null) {
                contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
                prefs.saveTree?.let { old -> runCatching { contentResolver.releasePersistableUriPermission(old, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION) } }
                prefs.saveTree = uri
                tree = uri
            }
        }
        Scaffold(
            topBar = {
                TopAppBar(
                    title = { Text("Settings") },
                    navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } },
                )
            },
        ) { pad ->
            LazyColumn(Modifier.padding(pad)) {
                item {
                    GroupCard(groupTitle(Group.Connection)) {
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
                            modifier = Modifier.clickable { startActivity(Intent(this@MainActivity, ScanActivity::class.java)) },
                        )
                    }
                }
                item {
                    GroupCard(groupTitle(Group.Camera)) {
                        CameraForPc()
                    }
                }
                item {
                    GroupCard(groupTitle(Group.Screen)) {
                        ControlFromPc()
                        NotificationsOnPc()
                        ScreenToPc()
                        val on = remember(resumed.intValue) { ControlService.instance != null }
                        val stuck = remember(resumed.intValue) { !on && ControlService.enabledInSettings(this@MainActivity) }
                        ListItem(
                            headlineContent = { Text("Accessibility permission") },
                            supportingContent = { Text("\u201c$APP_NAME control\u201d lets your PC tap and type here. Without it your PC cannot control this phone") },
                            trailingContent = { Text(if (on) "On" else if (stuck) "Stopped" else "Off", color = MaterialTheme.colorScheme.primary) },
                            modifier = Modifier.clickable { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) },
                        )
                    }
                }
                item {
                    GroupCard(groupTitle(Group.Files)) {
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
                                        tree?.let { runCatching { contentResolver.releasePersistableUriPermission(it, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION) } }
                                        prefs.saveTree = null
                                        tree = null
                                    }) { Text("Reset") }
                                }
                            },
                            modifier = Modifier.clickable { pickFolder.launch(null) },
                        )
                    }
                }
                item {
                    GroupCard(groupTitle(Group.Settings)) {
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
                        Text(
                            "Android only lets apps read the clipboard while they are open. So copies made on this phone are shared when you open $APP_NAME or its Quick Settings panel (long-press the tile). Copies from your PC arrive any time.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                        )
                        ListItem(
                            headlineContent = { Text("$APP_NAME") },
                            supportingContent = { Text(if (Bridge.running.value) "On: your PC can reach this phone. Quick Settings tile turns it off" else "Off: your PC cannot reach this phone") },
                            trailingContent = { Switch(Bridge.running.value, { setBridgeEnabled(this@MainActivity, it) }) },
                        )
                        val unrestricted = remember(resumed.intValue) { mutableStateOf(isBatteryUnrestricted(this@MainActivity)) }
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
                                    startActivity(
                                        Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)
                                            .setData(Uri.parse("package:$packageName")),
                                    )
                                } else {
                                    startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
                                }
                            },
                        )
                        ListItem(
                            headlineContent = { Text("Hide the status notification") },
                            supportingContent = { Text("Android needs it while $APP_NAME runs in the background. You can turn it off for this app; $APP_NAME keeps working") },
                            modifier = Modifier.clickable {
                                startActivity(
                                    Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
                                        .putExtra(Settings.EXTRA_APP_PACKAGE, packageName)
                                        .putExtra(Settings.EXTRA_CHANNEL_ID, "status"),
                                )
                            },
                        )
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
private fun GroupCard(title: String, content: @Composable () -> Unit) {
    Section(title)
    Card(
        shape = RoundedCornerShape(16.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 12.dp),
    ) {
        Column(Modifier.padding(vertical = 16.dp)) { content() }
    }
}

/** A home tile: accent container when [on], dimmed when disabled; tappable only when it has an [action]. */
@Composable
private fun HomeTileCard(tile: HomeTile, action: (() -> Unit)?, modifier: Modifier = Modifier) {
    val colors = if (tile.on) {
        CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.primaryContainer, contentColor = MaterialTheme.colorScheme.onPrimaryContainer)
    } else {
        CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer, contentColor = MaterialTheme.colorScheme.onSurface)
    }
    val body: @Composable () -> Unit = {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(tile.label, style = MaterialTheme.typography.titleMedium)
            Text(
                if (tile.group == Group.Files) "Share from any app" else if (tile.on) "On" else "Off",
                style = MaterialTheme.typography.bodySmall,
                color = LocalContentColor.current.copy(alpha = 0.7f),
            )
        }
    }
    val shape = RoundedCornerShape(16.dp)
    val m = modifier.alpha(if (tile.enabled) 1f else 0.5f)
    if (action != null && tile.enabled) Card(onClick = action, shape = shape, colors = colors, modifier = m) { body() }
    else Card(shape = shape, colors = colors, modifier = m) { body() }
}

/** Phone and PC joined by the link line: the design's signature element. */
@Composable
private fun LinkIndicator(phone: String, pc: String?, running: Boolean) {
    val badge = MaterialTheme.colorScheme.primaryContainer
    val onBadge = MaterialTheme.colorScheme.onPrimaryContainer
    val line = MaterialTheme.colorScheme.outline
    Row(verticalAlignment = Alignment.Top, modifier = Modifier.fillMaxWidth()) {
        DeviceBadge(phone, badge, onBadge, isPhone = true)
        Box(Modifier.weight(1f).height(44.dp), contentAlignment = Alignment.Center) {
            Canvas(Modifier.fillMaxWidth().height(2.dp).padding(horizontal = 8.dp)) {
                drawLine(
                    if (pc != null && running) line else line.copy(alpha = 0.35f),
                    Offset(0f, size.height / 2), Offset(size.width, size.height / 2),
                    strokeWidth = 4f, pathEffect = PathEffect.dashPathEffect(floatArrayOf(4f, 10f)),
                )
            }
        }
        DeviceBadge(pc ?: "Not paired", badge, onBadge, isPhone = false)
    }
}

@Composable
private fun DeviceBadge(name: String, bg: Color, fg: Color, isPhone: Boolean) {
    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Box(Modifier.size(44.dp).clip(RoundedCornerShape(14.dp)).background(bg), contentAlignment = Alignment.Center) {
            Canvas(Modifier.size(if (isPhone) 14.dp else 20.dp, if (isPhone) 22.dp else 14.dp)) {
                drawRoundRect(fg, cornerRadius = CornerRadius(6f, 6f), style = Stroke(width = 4f))
            }
        }
        Text(name, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.width(110.dp), textAlign = TextAlign.Center)
    }
}

@Composable
private fun KindBadge(kind: RecentItem.Kind) {
    val label = when (kind) {
        RecentItem.Kind.File -> "F"
        RecentItem.Kind.Clipboard -> "C"
        RecentItem.Kind.Link -> "L"
    }
    Box(Modifier.size(40.dp).clip(RoundedCornerShape(12.dp)).background(MaterialTheme.colorScheme.secondaryContainer), contentAlignment = Alignment.Center) {
        Text(label, color = MaterialTheme.colorScheme.onSecondaryContainer, fontWeight = FontWeight.Medium)
    }
}
