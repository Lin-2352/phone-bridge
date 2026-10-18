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
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.pb_ffi.PbNode

private const val TAG = "PhoneBridge"

/**
 * UI over the [PbService]-owned node. Screens follow design v2: Home (link
 * status, Share clipboard, recent), Settings, and the pairing code check.
 *
 * Test hooks (debug use over adb): intent extras `pair_code`, `confirm` ("yes"/"no"),
 * `send_text`. They drive exactly the same code paths as the UI.
 */
class MainActivity : ComponentActivity() {
    private val sas = mutableStateOf<String?>(null)
    private val message = mutableStateOf<String?>(null)
    private lateinit var prefs: Prefs

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        prefs = Prefs(this)
        ThemeChoice.mode.intValue = prefs.theme
        if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
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
        handleIntent(intent)
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
        Bridge.scope.launch {
            val node = awaitNode() ?: return@launch
            intent.getStringExtra("pair_code")?.let { join(node, it) }
            intent.getStringExtra("confirm")?.let { confirm(node, it == "yes") }
            intent.getStringExtra("send_text")?.let { t ->
                val peer = node.peers().firstOrNull() ?: return@let
                runCatching { node.sendText(peer.id, t, false) }.onSuccess { Log.i(TAG, "SENT ${t.length} chars to ${peer.name}") }
            }
        }
    }

    private suspend fun awaitNode(): PbNode? {
        repeat(100) {
            Bridge.node?.let { return it }
            delay(100)
        }
        return null
    }

    private suspend fun join(node: PbNode, code: String) {
        try {
            val s = node.joinPairing(code.trim())
            Log.i(TAG, "SAS=$s")
            sas.value = s
        } catch (e: Throwable) {
            Log.i(TAG, "Pairing failed: ${e.message}")
            message.value = "Pairing failed: ${e.message}"
        }
    }

    private suspend fun confirm(node: PbNode, accept: Boolean) {
        sas.value = null
        try {
            val name = node.confirmPairing(accept)
            Log.i(TAG, "PAIRED with $name")
            Bridge.linked.value = name
            PbService.refresh(this)
        } catch (e: Throwable) {
            Log.i(TAG, "Pairing not completed: ${e.message}")
            message.value = "Pairing not completed"
        }
    }

    @Composable
    private fun App() {
        var screen by remember { mutableStateOf("home") }
        val scope = rememberCoroutineScope()
        when (screen) {
            "settings" -> {
                BackHandler { screen = "home" }
                SettingsScreen(onBack = { screen = "home" })
            }
            else -> HomeScreen(onSettings = { screen = "settings" })
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
                confirmButton = { Button(onClick = { scope.launch { Bridge.node?.let { confirm(it, true) } } }) { Text("Yes, pair") } },
                dismissButton = { OutlinedButton(onClick = { scope.launch { Bridge.node?.let { confirm(it, false) } } }) { Text("No") } },
            )
        }
        message.value?.let { m ->
            AlertDialog(onDismissRequest = { message.value = null }, text = { Text(m) }, confirmButton = { TextButton(onClick = { message.value = null }) { Text("OK") } })
        }
    }

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    private fun HomeScreen(onSettings: () -> Unit) {
        var clipOn by remember { mutableStateOf(prefs.clipboardSync) }
        val pc = Bridge.linked.value
        val running = Bridge.running.value
        Scaffold(
            topBar = {
                TopAppBar(
                    title = { Text("Phone Bridge") },
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
                                !running -> "Phone Bridge is off. Your PC cannot reach this phone."
                                pc != null -> "Linked. Anything you share to Phone Bridge arrives on your PC."
                                else -> "Not paired yet. On your PC, open Phone Bridge and choose Pair new device, then tap Pair below."
                            },
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (!running) {
                            Button(onClick = { setBridgeEnabled(this@MainActivity, true) }) { Text("Turn on") }
                        } else if (pc == null) {
                            Button(onClick = { startActivity(Intent(this@MainActivity, ScanActivity::class.java)) }) { Text("Pair: scan QR from PC") }
                        }
                    }
                }
                item {
                    ListItem(
                        headlineContent = { Text("Share clipboard") },
                        supportingContent = { Text("Copy here, paste on your PC, and back") },
                        trailingContent = {
                            Switch(checked = clipOn, onCheckedChange = {
                                clipOn = it
                                prefs.clipboardSync = it
                                Bridge.node?.setClipboardSync(it)
                            })
                        },
                    )
                }
                item { Text("Recent", color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(start = 16.dp, top = 8.dp)) }
                if (Bridge.recent.isEmpty()) {
                    item { Text("Nothing yet. Share a photo or file to Phone Bridge from any app.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(16.dp)) }
                }
                items(Bridge.recent) { r ->
                    ListItem(
                        headlineContent = { Text(r.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        supportingContent = { Text(r.subtitle) },
                        leadingContent = { KindBadge(r.kind) },
                        modifier = Modifier.clickable(enabled = r.uri != null) {
                            r.uri?.let { startActivity(Intent(Intent.ACTION_VIEW).setData(it).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)) }
                        },
                    )
                }
            }
        }
    }

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    private fun SettingsScreen(onBack: () -> Unit) {
        var clipOn by remember { mutableStateOf(prefs.clipboardSync) }
        var sens by remember { mutableStateOf(prefs.sendSensitive) }
        var auto by remember { mutableStateOf(prefs.autoAccept) }
        var tree by remember { mutableStateOf(prefs.saveTree) }
        val peers = remember { Bridge.node?.peers().orEmpty() }
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
                item { Section("Appearance") }
                item {
                    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
                        listOf("System", "Light", "Dark").forEachIndexed { i, label ->
                            SegmentedButton(
                                selected = ThemeChoice.mode.intValue == i,
                                onClick = { ThemeChoice.mode.intValue = i; prefs.theme = i },
                                shape = SegmentedButtonDefaults.itemShape(i, 3),
                            ) { Text(label) }
                        }
                    }
                }
                item { Section("Clipboard") }
                item {
                    ListItem(
                        headlineContent = { Text("Share clipboard") },
                        supportingContent = { Text("Share what you copy with your paired PC") },
                        trailingContent = { Switch(clipOn, { clipOn = it; prefs.clipboardSync = it; Bridge.node?.setClipboardSync(it) }) },
                    )
                }
                item {
                    ListItem(
                        headlineContent = { Text("Passwords and codes") },
                        supportingContent = { Text("Off: text an app marks sensitive never leaves this phone. On: it is sent, kept out of clipboard history and hidden in notifications on the PC") },
                        trailingContent = { Switch(sens, { sens = it; prefs.sendSensitive = it }, enabled = clipOn) },
                    )
                }
                item {
                    Text(
                        "Android only lets apps read the clipboard while they are open. So copies made on this phone are shared when you open Phone Bridge or its Quick Settings panel (long-press the tile). Copies from your PC arrive any time.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                    )
                }
                item { Section("Sharing") }
                item {
                    ListItem(
                        headlineContent = { Text("Accept from my devices automatically") },
                        supportingContent = { Text("Only paired devices can send") },
                        trailingContent = { Switch(auto, { auto = it; prefs.autoAccept = it; Bridge.node?.setAutoAccept(it) }) },
                    )
                }
                item {
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
                item { Section("Background") }
                item {
                    ListItem(
                        headlineContent = { Text("Phone Bridge") },
                        supportingContent = { Text(if (Bridge.running.value) "On: your PC can reach this phone. Quick Settings tile turns it off" else "Off: your PC cannot reach this phone") },
                        trailingContent = { Switch(Bridge.running.value, { setBridgeEnabled(this@MainActivity, it) }) },
                    )
                }
                item {
                    ListItem(
                        headlineContent = { Text("Hide the status notification") },
                        supportingContent = { Text("Android needs it while Phone Bridge runs in the background. You can turn it off for this app; Phone Bridge keeps working") },
                        modifier = Modifier.clickable {
                            startActivity(
                                Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
                                    .putExtra(Settings.EXTRA_APP_PACKAGE, packageName)
                                    .putExtra(Settings.EXTRA_CHANNEL_ID, "status"),
                            )
                        },
                    )
                }
                item { Section("Devices") }
                items(peers) { p ->
                    ListItem(
                        headlineContent = { Text(p.name) },
                        supportingContent = { Text("Fingerprint ${p.fingerprint.take(9)}") },
                        trailingContent = {
                            TextButton(onClick = {
                                Bridge.node?.unpair(p.id)
                                Bridge.linked.value = Bridge.node?.peers()?.firstOrNull()?.name
                                PbService.refresh(this@MainActivity)
                                onBack()
                            }) { Text("Remove", color = MaterialTheme.colorScheme.error) }
                        },
                    )
                }
                item {
                    ListItem(
                        headlineContent = { Text("Pair a new PC", color = MaterialTheme.colorScheme.primary) },
                        modifier = Modifier.clickable { startActivity(Intent(this@MainActivity, ScanActivity::class.java)) },
                    )
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
