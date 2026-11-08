package dev.phonebridge.app

import dev.phonebridge.app.ui.LocalResumeCount
import dev.phonebridge.app.ui.RelayNavHost
import dev.phonebridge.app.ui.RelayTheme
import dev.phonebridge.app.ui.TAB_EXTRA
import dev.phonebridge.app.ui.Tab
import dev.phonebridge.app.ui.ThemeChoice
import dev.phonebridge.app.ui.loadFrom
import dev.phonebridge.app.ui.isDark
import dev.phonebridge.app.ui.startTab
import dev.phonebridge.app.ui.pairing.PairPhase
import dev.phonebridge.app.ui.pairing.PairingFlow
import dev.phonebridge.app.ui.pairing.pairingProgressText
import dev.phonebridge.app.ui.Amber
import dev.phonebridge.app.ui.AmberDark
import dev.phonebridge.app.widget.RelayWidget
import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color as AColor
import android.os.Build
import android.os.Bundle
import android.util.Log
import androidx.fragment.app.FragmentActivity
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
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.ui.semantics.liveRegion
import dev.phonebridge.app.ui.components.animationsEnabled
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.sp
import dev.phonebridge.app.devices.isLargeScreen
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.draganddrop.dragAndDropTarget
import androidx.compose.ui.draganddrop.toAndroidDragEvent
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
class MainActivity : FragmentActivity() {
    private val sas = Bridge.sas
    private val message = Bridge.pairMessage
    private lateinit var prefs: Prefs
    /** Bumped on every resume: rows showing system settings re-read them. */
    private val resumed = mutableIntStateOf(0)
    /** The tab a launch or a new intent asked for; [tabRequest] changes with each ask. */
    private val requestedTab = mutableStateOf(Tab.Home)
    private val tabRequest = mutableIntStateOf(0)
    private val debuggable get() = applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0

    override fun dispatchTouchEvent(ev: android.view.MotionEvent): Boolean =
        if (sas.value != null && isObscured(ev.flags)) true else super.dispatchTouchEvent(ev)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        AppLog.init(this)
        prefs = Prefs(this)
        // Kept across rotation and process death so a handled request is not replayed.
        tabRequest.intValue = savedInstanceState?.getInt(TAB_REQUEST) ?: 0
        if (savedInstanceState == null) requestedTab.value = startTab(intent, debuggable)
        Timeline.init(this)
        ThemeChoice.loadFrom(prefs)
        Bridge.allowControl.value = prefs.allowControl
        // A process killed without onDestroy left the widget's last state behind: opening the app corrects it.
        RelayWidget.update(this)
        // Only on a fresh start: a recreation (dark mode, font size) must not ask again.
        if (savedInstanceState == null && Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        // Relay only talks to your own devices on this network; Android 17 asks first once Relay targets it.
        if (savedInstanceState == null && needsLocalNetworkRequest(
                Build.VERSION.SDK_INT,
                applicationInfo.targetSdkVersion,
                checkSelfPermission(ACCESS_LOCAL_NETWORK) == PackageManager.PERMISSION_GRANTED,
            )
        ) {
            requestPermissions(arrayOf(ACCESS_LOCAL_NETWORK), 2)
        }
        honourUserStop(this)
        // A stop from the system's active-apps list can land mid-share, and with Relay now off no service
        // starts to put the volume back. A live share is left alone.
        if (!AudioShare.sessions.isActive(dev.phonebridge.app.audio.AudioDirection.PHONE_TO_PC)) PhoneSpeaker.restore(this)
        // With the identity lost, Home shows the reset card; a start would only fail and notify again.
        if (startsOnOpen(prefs.enabled, prefs.identityLost)) startForegroundService(Intent(this, PbService::class.java))
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

    override fun onStart() {
        super.onStart()
        Bridge.uiVisible.incrementAndGet()
        Bridge.refreshActivity()
    }

    override fun onStop() {
        Bridge.uiVisible.updateAndGet { (it - 1).coerceAtLeast(0) }
        Bridge.refreshActivity()
        super.onStop()
    }

    override fun onResume() {
        super.onResume()
        // A visible window may set the volume: finish a restore Android 17 refused in the background.
        if (Prefs(this).pendingVolumeRestore) {
            val app = applicationContext
            dev.phonebridge.app.audio.VolumeJobs.shared.submit { PhoneSpeaker.restore(app) }
            Prefs(this).pendingVolumeRestore = false
        }
        AppLog.permissionsChanged(this)
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
        if (intent.hasExtra("audio_test_tone")) AudioShare.debugTone = intent.getBooleanExtra("audio_test_tone", false)
        if (!intent.hasExtra("pair_code") && !intent.hasExtra("confirm") && !intent.hasExtra("send_text")) return
        Bridge.scope.launch {
            val node = Bridge.awaitNode() ?: return@launch
            intent.getStringExtra("pair_code")?.let { PairingFlow.start(this@MainActivity, it) }
            intent.getStringExtra("confirm")?.let { PairingFlow.confirm(applicationContext, it == "yes") }
            intent.getStringExtra("send_text")?.let { t ->
                val peer = node.peers().firstOrNull() ?: return@let
                runCatching { node.sendText(peer.id, t, false) }.onSuccess { Log.i(TAG, "SENT ${t.length} chars to ${peer.name}") }
            }
        }
    }

    @OptIn(ExperimentalFoundationApi::class)
    @Composable
    private fun App() {
        val scope = rememberCoroutineScope()
        // On a large window (tablet, foldable, desktop mode) files can be dragged in from another window and are
        // sent to the default PC; a phone-sized window takes no drops.
        val large = isLargeScreen(androidx.compose.ui.platform.LocalConfiguration.current.screenWidthDp)
        val dropReceiver = androidx.compose.runtime.remember { dev.phonebridge.app.devices.DropReceiver(this@MainActivity) }
        androidx.compose.foundation.layout.Box(
            if (large) {
                androidx.compose.ui.Modifier.dragAndDropTarget(
                    shouldStartDragAndDrop = { it.toAndroidDragEvent().clipDescription?.hasMimeType("*/*") == true },
                    target = dropReceiver,
                )
            } else {
                androidx.compose.ui.Modifier
            },
        ) {
            RelayNavHost(requestedTab.value, tabRequest.intValue)
        }
        sas.value?.let { code ->
            // The comparison is a consent prompt: no overlays, no remote taps while it shows.
            androidx.compose.runtime.DisposableEffect(Unit) {
                if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.S) window.setHideOverlayWindows(true)
                ConsentGuard.begin("sas", android.os.SystemClock.elapsedRealtime())
                onDispose {
                    if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.S) window.setHideOverlayWindows(false)
                    ConsentGuard.end("sas")
                }
            }
            // The guard lapses after 120 s (a lost prompt must not block input for good); the
            // comparison can stay open longer, so it renews itself while shown.
            androidx.compose.runtime.LaunchedEffect(Unit) {
                while (true) {
                    kotlinx.coroutines.delay(60_000)
                    ConsentGuard.begin("sas", android.os.SystemClock.elapsedRealtime())
                }
            }
            val amber = if (isDark()) AmberDark else Amber
            RelayDialog(
                // The PC's name is not known until pairing completes, so the heading stays generic.
                title = "Pair with this PC?",
                onDismissRequest = {},
                confirmText = "Yes, it matches",
                onConfirm = { scope.launch { PairingFlow.confirm(applicationContext, true) } },
                dismissText = "No",
                onDismiss = { scope.launch { PairingFlow.confirm(applicationContext, false) } },
            ) {
                Text("Check that your PC shows this code.", style = MaterialTheme.typography.bodyMedium)
                Text(
                    groupCode(code),
                    fontSize = 40.sp,
                    fontWeight = FontWeight.Medium,
                    letterSpacing = 2.sp,
                    // The digits carry the contrast (onSurface on the tint, tested in ContrastTest); the amber is decoration.
                    color = MaterialTheme.colorScheme.onSurface,
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
        when (val ph = PairingFlow.phase.value) {
            is PairPhase.Joining -> PairingProgress(ph.startedAt)
            is PairPhase.Failed -> RelayDialog(
                title = "Pairing did not work",
                onDismissRequest = { PairingFlow.dismiss() },
                confirmText = "Scan again",
                onConfirm = { PairingFlow.dismiss(); startActivity(Intent(this@MainActivity, ScanActivity::class.java)) },
                dismissText = "Close",
                onDismiss = { PairingFlow.dismiss() },
            ) { Text(ph.text, style = MaterialTheme.typography.bodyMedium) }
            PairPhase.Idle -> {}
        }
        message.value?.let { m ->
            RelayDialog(title = null, onDismissRequest = { message.value = null }, confirmText = "OK", onConfirm = { message.value = null }) { Text(m, style = MaterialTheme.typography.bodyMedium) }
        }
    }
}

/** Shown from the scan until the code check or an error: what is happening, with a way out. */
@Composable
private fun PairingProgress(startedAt: Long) {
    var elapsed by remember { mutableLongStateOf(0L) }
    LaunchedEffect(startedAt) {
        while (true) {
            elapsed = android.os.SystemClock.elapsedRealtime() - startedAt
            kotlinx.coroutines.delay(1000)
        }
    }
    val animate = animationsEnabled(androidx.compose.ui.platform.LocalContext.current)
    RelayDialog(
        title = "Pairing with your PC…",
        onDismissRequest = {},
        confirmText = "Cancel",
        onConfirm = { PairingFlow.cancel() },
    ) {
        if (animate) androidx.compose.material3.LinearProgressIndicator(Modifier.fillMaxWidth())
        Text(
            pairingProgressText(elapsed),
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.semantics { liveRegion = androidx.compose.ui.semantics.LiveRegionMode.Polite },
        )
    }
}

private const val TAB_REQUEST = "tab_request"