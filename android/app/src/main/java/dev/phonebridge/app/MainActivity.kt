package dev.phonebridge.app

import dev.phonebridge.app.ui.LocalResumeCount
import dev.phonebridge.app.ui.RelayNavHost
import dev.phonebridge.app.ui.RelayTheme
import dev.phonebridge.app.ui.TAB_EXTRA
import dev.phonebridge.app.ui.Tab
import dev.phonebridge.app.ui.ThemeChoice
import dev.phonebridge.app.ui.isDark
import dev.phonebridge.app.ui.startTab
import dev.phonebridge.app.ui.Amber
import dev.phonebridge.app.ui.AmberDark
import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color as AColor
import android.os.Build
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch

private const val TAG = "PhoneBridge"

/**
 * Host for the UI over the [PbService]-owned node: the theme, the four-tab [RelayNavHost]
 * (screens live under ui/), and the pairing code check, which shows over any tab.
 *
 * Test hooks (debug use over adb): intent extras `pair_code`, `confirm` ("yes"/"no"),
 * `send_text`. They drive exactly the same code paths as the UI. `tab` (a [Tab] route)
 * opens that tab, at launch or on a new intent.
 */
class MainActivity : ComponentActivity() {
    private val sas = Bridge.sas
    private val message = Bridge.pairMessage
    private lateinit var prefs: Prefs
    /** Bumped on every resume: rows showing system settings re-read them. */
    private val resumed = mutableIntStateOf(0)
    /** The tab a launch or a new intent asked for; [tabRequest] changes with each ask. */
    private val requestedTab = mutableStateOf(Tab.Home)
    private val tabRequest = mutableIntStateOf(0)
    private val debuggable get() = applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        prefs = Prefs(this)
        // Kept across rotation and process death so a handled request is not replayed.
        tabRequest.intValue = savedInstanceState?.getInt(TAB_REQUEST) ?: 0
        if (savedInstanceState == null) requestedTab.value = startTab(intent, debuggable)
        Timeline.init(this)
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
            RelayTheme {
                val dark = isDark()
                LaunchedEffect(dark) {
                    val bars = if (dark) SystemBarStyle.dark(AColor.TRANSPARENT) else SystemBarStyle.light(AColor.TRANSPARENT, AColor.TRANSPARENT)
                    enableEdgeToEdge(statusBarStyle = bars, navigationBarStyle = bars)
                }
                Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) {
                    CompositionLocalProvider(LocalResumeCount provides resumed.intValue) { App() }
                }
            }
        }
        if (savedInstanceState == null) handleIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        resumed.intValue++
        // Picture access may have been granted or taken away in Android's settings.
        ScreenshotWatcher.sync(this)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        // Only an explicit `tab` moves the bar: other re-launches (notification, pairing) keep the tab.
        if (debuggable && intent.hasExtra(TAB_EXTRA)) {
            requestedTab.value = startTab(intent, debuggable)
            tabRequest.intValue++
        }
        handleIntent(intent)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putInt(TAB_REQUEST, tabRequest.intValue)
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
        val scope = rememberCoroutineScope()
        RelayNavHost(requestedTab.value, tabRequest.intValue)
        sas.value?.let { code ->
            val amber = if (isDark()) AmberDark else Amber
            RelayDialog(
                // The PC's name is not known until pairing completes, so the heading stays generic.
                title = "Pair with this PC?",
                onDismissRequest = {},
                confirmText = "Yes, it matches",
                onConfirm = { scope.launch { Bridge.confirm(applicationContext, true) } },
                dismissText = "No",
                onDismiss = { scope.launch { Bridge.confirm(applicationContext, false) } },
            ) {
                Text("Check that your PC shows this code.", style = MaterialTheme.typography.bodyMedium)
                Text(
                    groupCode(code),
                    fontSize = 40.sp,
                    fontWeight = FontWeight.Medium,
                    letterSpacing = 2.sp,
                    color = amber,
                    textAlign = TextAlign.Center,
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(Radius.card))
                        .background(amber.copy(alpha = 0.16f))
                        .padding(vertical = Spacing.lg)
                        .semantics { contentDescription = "Pairing code ${code.toList().joinToString(" ")}" },
                )
                Text("If the codes are different, choose No. Someone may be trying to intercept the connection.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        message.value?.let { m ->
            RelayDialog(title = null, onDismissRequest = { message.value = null }, confirmText = "OK", onConfirm = { message.value = null }) { Text(m, style = MaterialTheme.typography.bodyMedium) }
        }
    }
}

private const val TAB_REQUEST = "tab_request"

/** One line in the Home screen's Recent list (until Home moves to the Timeline screen). */
data class RecentItem(val title: String, val subtitle: String, val kind: Kind, val uri: String? = null) {
    enum class Kind { File, Clipboard, Link }
}

/** A timeline entry in the Recent list's shape: metadata only, as stored. */
fun recentItem(e: TlEntry) = RecentItem(
    title = summary(e),
    subtitle = if (e.dir == TlDir.ToPc) "Sent to ${e.peer}" else "From ${e.peer}",
    kind = when (e.kind) {
        TlKind.File -> RecentItem.Kind.File
        TlKind.Link -> RecentItem.Kind.Link
        else -> RecentItem.Kind.Clipboard
    },
    uri = e.uri,
)
