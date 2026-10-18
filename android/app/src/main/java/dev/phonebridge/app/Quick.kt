package dev.phonebridge.app

import android.app.Activity
import android.app.PendingIntent
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.util.Log
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
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
    if (on) {
        ctx.startForegroundService(Intent(ctx, PbService::class.java))
    } else {
        ctx.stopService(Intent(ctx, PbService::class.java))
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
    val text = clip.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(ctx)?.toString()
    return if (text.isNullOrEmpty()) null else text to sensitive
}

/**
 * Quick Settings tile "Phone Bridge": tap turns the whole app on or off.
 * Long-press opens [QuickPanelActivity] (QS_TILE_PREFERENCES).
 */
class BridgeTileService : TileService() {
    override fun onStartListening() {
        val on = Prefs(this).enabled
        qsTile?.apply {
            state = if (on) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
            if (Build.VERSION.SDK_INT >= 29) {
                subtitle = when {
                    !on -> "Off"
                    Bridge.linked.value != null -> Bridge.linked.value
                    else -> "On"
                }
            }
            updateTile()
        }
    }

    override fun onClick() {
        val on = !Prefs(this).enabled
        if (!on) {
            setBridgeEnabled(this, false)
        } else {
            try {
                setBridgeEnabled(this, true)
            } catch (t: Throwable) {
                // Some versions refuse a foreground start from the tile: go through
                // an invisible activity, which is always allowed to start it.
                Log.i(TAG, "TILE start via activity: $t")
                val i = Intent(this, StarterActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                if (Build.VERSION.SDK_INT >= 34) {
                    startActivityAndCollapse(PendingIntent.getActivity(this, 0, i, PendingIntent.FLAG_IMMUTABLE))
                } else {
                    @Suppress("DEPRECATION")
                    startActivityAndCollapse(i)
                }
            }
        }
        Log.i(TAG, "TILE Phone Bridge ${if (on) "on" else "off"}")
        onStartListening()
    }
}

/** Starts the service from the foreground, then disappears. */
class StarterActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setBridgeEnabled(this, true)
        finish()
    }
}

/**
 * Long-press on the tile: the everyday switches in a small sheet, without
 * opening the whole app. Being focused, it can also read the clipboard.
 */
class QuickPanelActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val prefs = Prefs(this)
        ThemeChoice.mode.intValue = prefs.theme
        setContent {
            PbTheme {
                Box(
                    Modifier
                        .fillMaxSize()
                        .background(Color.Black.copy(alpha = 0.32f))
                        .clickable(remember { MutableInteractionSource() }, null) { finish() },
                    contentAlignment = Alignment.BottomCenter,
                ) {
                    Panel(prefs)
                }
            }
        }
    }

    @Composable
    private fun Panel(prefs: Prefs) {
        var on by remember { mutableStateOf(prefs.enabled) }
        var clip by remember { mutableStateOf(prefs.clipboardSync) }
        var sens by remember { mutableStateOf(prefs.sendSensitive) }
        var auto by remember { mutableStateOf(prefs.autoAccept) }
        val pick = rememberLauncherForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris: List<Uri> ->
            if (uris.isNotEmpty()) {
                startActivity(
                    Intent(this, ShareActivity::class.java)
                        .setAction(Intent.ACTION_SEND_MULTIPLE)
                        .putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList(uris))
                        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION),
                )
            }
            finish()
        }
        val itemColors = ListItemDefaults.colors(containerColor = Color.Transparent)
        Column(
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp))
                .background(MaterialTheme.colorScheme.surfaceContainer)
                .clickable(remember { MutableInteractionSource() }, null) {}
                .navigationBarsPadding()
                .padding(vertical = 12.dp),
        ) {
            Text(
                "Phone Bridge",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
            )
            ListItem(
                colors = itemColors,
                headlineContent = { Text("Phone Bridge") },
                supportingContent = {
                    Text(if (on) Bridge.linked.value?.let { "On, paired with $it" } ?: "On" else "Off. Your PC cannot reach this phone")
                },
                trailingContent = { Switch(on, { on = it; setBridgeEnabled(this@QuickPanelActivity, it) }) },
            )
            ListItem(
                colors = itemColors,
                headlineContent = { Text("Universal Clipboard") },
                supportingContent = { Text("Copy on one device, paste on the other") },
                trailingContent = { Switch(clip, { clip = it; prefs.clipboardSync = it; Bridge.node?.setClipboardSync(it) }) },
            )
            ListItem(
                colors = itemColors,
                headlineContent = { Text("Also send passwords and codes") },
                supportingContent = { Text("Text apps mark as sensitive. Kept out of clipboard history on the PC") },
                trailingContent = { Switch(sens, { sens = it; prefs.sendSensitive = it }, enabled = clip) },
            )
            ListItem(
                colors = itemColors,
                headlineContent = { Text("Accept files automatically") },
                supportingContent = { Text("From your paired PC only") },
                trailingContent = { Switch(auto, { auto = it; prefs.autoAccept = it; Bridge.node?.setAutoAccept(it) }) },
            )
            HorizontalDivider(Modifier.padding(horizontal = 24.dp, vertical = 8.dp))
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                FilledTonalButton(onClick = { sendClipboardNow(prefs) }, enabled = on) { Text("Send clipboard") }
                FilledTonalButton(onClick = { pick.launch("*/*") }, enabled = on) { Text("Send files") }
                FilledTonalButton(onClick = {
                    startActivity(Intent(this@QuickPanelActivity, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    finish()
                }) { Text("Open") }
            }
        }
    }

    private fun sendClipboardNow(prefs: Prefs) {
        val node = Bridge.node
        val peer = Bridge.firstPeer()
        val clip = readClipboard(this)
        val msg = when {
            node == null || peer == null -> "Pair Phone Bridge with your PC first"
            clip == null -> "Clipboard is empty"
            clip.second && !prefs.sendSensitive -> "Not sent: it is a password or code. Allow that in this panel first"
            else -> {
                val text = clip.first
                prefs.lastClip = text.hashCode()
                Bridge.scope.launch {
                    runCatching { node.sendText(peer.id, text, false) }
                        .onSuccess { Log.i(TAG, "CLIPSEND ${text.length} chars to ${peer.name}") }
                        .onFailure { Log.e(TAG, "CLIPSEND failed: $it") }
                }
                "Clipboard sent to ${peer.name}"
            }
        }
        Toast.makeText(this, msg, Toast.LENGTH_SHORT).show()
        finish()
    }
}
