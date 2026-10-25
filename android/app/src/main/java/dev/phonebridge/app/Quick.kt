package dev.phonebridge.app

import android.app.Activity
import android.app.PendingIntent
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.util.Log
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.TextButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

private const val TAG = "PhoneBridge"

/** Turn Phone Bridge on (service up, phone reachable) or off (service and notification gone). */
fun setBridgeEnabled(ctx: Context, on: Boolean) {
    Prefs(ctx).enabled = on
    Bridge.startingSince = if (on && !Bridge.running.value) android.os.SystemClock.uptimeMillis() else 0L
    if (on) {
        ctx.startForegroundService(Intent(ctx, PbService::class.java))
    } else {
        ctx.stopService(Intent(ctx, PbService::class.java))
        cancelRecovery(ctx)
        ctx.getSystemService(android.app.NotificationManager::class.java).cancel(STOPPED_ID)
    }
    refreshTile(ctx)
}

fun refreshTile(ctx: Context) {
    TileService.requestListeningState(ctx, ComponentName(ctx, BridgeTileService::class.java))
}

/** Clipboard text and whether its app marked it sensitive. Only works while we have focus. */
fun readClipboard(ctx: Context): Pair<String, Boolean>? {
    val clip = ctx.getSystemService(ClipboardManager::class.java).primaryClip ?: return null
    val sensitive = clip.description?.extras?.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE) == true
    // Plain text only: coerceToText would read a copied file's whole content
    // through its provider, on the main thread.
    val text = clip.takeIf { it.itemCount > 0 }?.getItemAt(0)?.text?.toString()?.take(60 * 1024)
    return if (text.isNullOrEmpty()) null else text to sensitive
}

/**
 * Quick Settings tile "Phone Bridge": tap turns the whole app on or off.
 * Long-press opens [QuickPanelActivity] (QS_TILE_PREFERENCES).
 */
class BridgeTileService : TileService() {
    private val main = android.os.Handler(android.os.Looper.getMainLooper())

    override fun onStartListening() = paint()

    override fun onStopListening() {
        main.removeCallbacksAndMessages(null)
    }

    /**
     * What is running (or starting), not the saved wish: a tap must do what
     * the tile shows. The service starts and stops asynchronously, and some
     * systems (realme) ignore later refresh requests, so the tile is painted
     * from the real state on every listen and again shortly after a tap.
     */
    private fun paint() {
        val on = Bridge.shownOn()
        qsTile?.apply {
            state = if (on) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
            if (Build.VERSION.SDK_INT >= 29) {
                subtitle = tileSubtitle(
                    wished = Prefs(this@BridgeTileService).enabled,
                    running = Bridge.running.value,
                    starting = on && !Bridge.running.value,
                    linked = Bridge.linked.value,
                )
            }
            updateTile()
        }
    }

    override fun onClick() {
        val plan = tileClick(Bridge.shownOn(), isLocked)
        Log.i(TAG, "TILE tap: $plan")
        val viaActivity = {
            val i = Intent(this, StarterActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (Build.VERSION.SDK_INT >= 34) {
                startActivityAndCollapse(PendingIntent.getActivity(this, 0, i, PendingIntent.FLAG_IMMUTABLE))
            } else {
                collapseAndStartLegacy(i)
            }
        }
        when (plan) {
            TileClick.Stop -> setBridgeEnabled(this, false)
            TileClick.StartViaActivity -> viaActivity()
            TileClick.StartDirect -> try {
                setBridgeEnabled(this, true)
            } catch (t: Throwable) {
                // Refused on the lock screen: start after the user unlocks.
                Log.i(TAG, "TILE direct start refused: $t")
                unlockAndRun { viaActivity() }
            }
        }
        paint()
        // While the panel keeps us bound, catch the service coming up or down.
        for (d in longArrayOf(500, 1500, 3000)) main.postDelayed({ paint() }, d)
    }
}

/** The only form API 29..33 has; the PendingIntent one is API 34+. */
@android.annotation.SuppressLint("StartActivityAndCollapseDeprecated")
@Suppress("DEPRECATION")
private fun TileService.collapseAndStartLegacy(i: Intent) = startActivityAndCollapse(i)

/** Starts the service from the foreground, then disappears. */
class StarterActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setBridgeEnabled(this, true)
        finish()
    }
}
/**
 * Tile detail panel, laid out like the system's own (DND, Bluetooth): blurred
 * shade behind, a card with close, title and master switch, the options, and
 * MORE. Opening it also shares the clipboard: Android lets only a focused
 * app read it, and this panel is focused.
 */
class QuickPanelActivity : ComponentActivity() {
    private lateinit var prefs: Prefs

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        prefs = Prefs(this)
        ThemeChoice.mode.intValue = prefs.theme
        if (Build.VERSION.SDK_INT >= 31) {
            window.addFlags(WindowManager.LayoutParams.FLAG_BLUR_BEHIND)
            window.attributes = window.attributes.apply { blurBehindRadius = 80 }
        }
        setContent {
            PbTheme {
                Box(
                    Modifier
                        .fillMaxSize()
                        .background(Color.Black.copy(alpha = 0.35f))
                        .clickable(remember { MutableInteractionSource() }, null) { finish() },
                    contentAlignment = Alignment.Center,
                ) {
                    Panel()
                }
            }
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) shareClipboardIfChanged(this, prefs)
    }

    @Composable
    private fun Panel() {
        // What is running (or starting), not the saved wish; follows the service.
        var on by remember(Bridge.running.value) { mutableStateOf(Bridge.shownOn()) }
        var clip by remember { mutableStateOf(prefs.clipboardSync) }
        var sens by remember { mutableStateOf(prefs.sendSensitive) }
        var auto by remember { mutableStateOf(prefs.autoAccept) }
        val itemColors = ListItemDefaults.colors(containerColor = Color.Transparent)
        Column(
            Modifier
                .padding(horizontal = 14.dp)
                .fillMaxWidth()
                .clip(RoundedCornerShape(32.dp))
                .background(MaterialTheme.colorScheme.surfaceContainer)
                .clickable(remember { MutableInteractionSource() }, null) {}
                .padding(vertical = 16.dp),
        ) {
            Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = { finish() }) { Icon(Icons.Filled.Close, "Close") }
                Text("$APP_NAME", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f).padding(start = 8.dp))
                Switch(on, { on = it; setBridgeEnabled(this@QuickPanelActivity, it) })
            }
            Text(
                when {
                    !on -> "Off. Your PC cannot reach this phone"
                    Bridge.linked.value != null -> "Linked with ${Bridge.linked.value}"
                    else -> "On, not paired yet"
                },
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(start = 72.dp, bottom = 8.dp),
            )
            ListItem(
                colors = itemColors,
                headlineContent = { Text("Share clipboard") },
                supportingContent = { Text("Copy on one device, paste on the other") },
                trailingContent = { Switch(clip, { clip = it; prefs.clipboardSync = it; Bridge.node?.setClipboardSync(it) }, enabled = on) },
            )
            ListItem(
                colors = itemColors,
                headlineContent = { Text("Passwords and codes") },
                supportingContent = { Text("Share them too. Kept out of clipboard history") },
                trailingContent = { Switch(sens, { sens = it; prefs.sendSensitive = it }, enabled = on && clip) },
            )
            ListItem(
                colors = itemColors,
                headlineContent = { Text("Accept files automatically") },
                supportingContent = { Text("From your paired PC only") },
                trailingContent = { Switch(auto, { auto = it; prefs.autoAccept = it; Bridge.node?.setAutoAccept(it) }, enabled = on) },
            )
            ListItem(
                colors = itemColors,
                headlineContent = { Text("Touchpad and keyboard") },
                supportingContent = { Text("Control your PC from this phone") },
                modifier = Modifier.clickable(enabled = on && Bridge.linked.value != null) {
                    startActivity(Intent(this@QuickPanelActivity, TouchpadActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    finish()
                },
            )
            TextButton(
                onClick = {
                    startActivity(Intent(this@QuickPanelActivity, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    finish()
                },
                modifier = Modifier.align(Alignment.CenterHorizontally).padding(top = 8.dp),
            ) { Text("MORE", style = MaterialTheme.typography.titleMedium) }
        }
    }
}

/** Phone -> PC Share clipboard, run whenever one of our windows gains focus. */
fun shareClipboardIfChanged(ctx: Context, prefs: Prefs) {
    if (!prefs.clipboardSync) return
    val (text, sensitive) = readClipboard(ctx) ?: return
    if (sensitive && !prefs.sendSensitive) return
    // Only when it changed since the last sync in either direction.
    if (text.hashCode() == prefs.lastClip) return
    if (!Bridge.shownOn()) return
    Bridge.scope.launch {
        // Opened cold: the node may still be starting.
        val node = Bridge.awaitNode() ?: return@launch
        runCatching { node.syncClipboard(text, sensitive) }
            .onSuccess {
                // Only now: a copy made while the PC was away is sent next time.
                prefs.lastClip = text.hashCode()
                Log.i(TAG, "CLIPSYNC sent ${text.length} chars")
            }
    }
}
