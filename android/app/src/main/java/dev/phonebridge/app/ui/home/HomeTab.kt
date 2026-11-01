package dev.phonebridge.app.ui.home

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.phonebridge.app.APP_NAME
import dev.phonebridge.app.Bridge
import dev.phonebridge.app.CameraShare
import dev.phonebridge.app.ClipImages
import dev.phonebridge.app.Group
import dev.phonebridge.app.HomeTile
import dev.phonebridge.app.Prefs
import dev.phonebridge.app.Radius
import dev.phonebridge.app.ScanActivity
import dev.phonebridge.app.ScreenShare
import dev.phonebridge.app.Timeline
import dev.phonebridge.app.TouchpadActivity
import dev.phonebridge.app.hasNotifyAccess
import dev.phonebridge.app.homeTiles
import dev.phonebridge.app.notifyAccessMissing
import dev.phonebridge.app.recentItem
import dev.phonebridge.app.screenshotAccessMissing
import dev.phonebridge.app.setBridgeEnabled
import dev.phonebridge.app.ui.LocalResumeCount
import dev.phonebridge.app.ui.components.KindBadge
import dev.phonebridge.app.ui.components.LinkIndicator
import dev.phonebridge.app.ui.settings.openNotifyAccess

/** The Home tab: for now the pre-v2 Home screen (link status, tiles, warnings, Recent) until it is rebuilt. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeTab(onSettings: () -> Unit) {
    val ctx = LocalContext.current
    val resumed = LocalResumeCount.current
    val prefs = remember { Prefs(ctx) }
    var clipOn by remember(resumed) { mutableStateOf(prefs.clipboardSync) }
    val pc = Bridge.linked.value
    val running = Bridge.running.value
    val notificationsOn = remember(resumed) { ctx.getSystemService(android.app.NotificationManager::class.java).areNotificationsEnabled() }
    val notifyMissing = remember(resumed) { notifyAccessMissing(prefs.notifyToPc, hasNotifyAccess(ctx)) }
    val shotsMissing = remember(resumed) { screenshotAccessMissing(prefs.shareScreenshots, ClipImages.hasImageAccess(ctx)) }
    val timeline by Timeline.entries.collectAsState()
    val recent = remember(timeline) { timeline.take(20).map(::recentItem) }
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
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(Radius.hero)).background(MaterialTheme.colorScheme.surfaceContainer).padding(20.dp),
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
                        Button(onClick = { setBridgeEnabled(ctx, true) }) { Text("Turn on") }
                    } else if (!Bridge.nodeReady.value) {
                        // Starting: nothing to tap yet.
                    } else if (pc == null) {
                        Button(onClick = { ctx.startActivity(Intent(ctx, ScanActivity::class.java)) }) { Text("Pair: scan QR from PC") }
                    } else {
                        Button(onClick = { ctx.startActivity(Intent(ctx, TouchpadActivity::class.java)) }) { Text("Use as touchpad and keyboard") }
                    }
                }
            }
            Bridge.controlledBy.value?.let { controller ->
                item {
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .padding(top = 8.dp)
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
            if (notifyMissing) {
                item {
                    ListItem(
                        headlineContent = { Text("Phone notifications are not reaching your PC") },
                        supportingContent = { Text("$APP_NAME needs notification access. Tap, then turn on $APP_NAME") },
                        modifier = Modifier.clickable { openNotifyAccess(ctx) },
                    )
                }
            }
            if (shotsMissing) {
                item {
                    ListItem(
                        headlineContent = { Text("Screenshots are not reaching your PC") },
                        supportingContent = { Text("$APP_NAME needs access to your photos to see new screenshots. Tap, then allow Photos") },
                        modifier = Modifier.clickable {
                            ctx.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).setData(Uri.parse("package:${ctx.packageName}")))
                        },
                    )
                }
            }
            if (!notificationsOn) {
                item {
                    ListItem(
                        headlineContent = { Text("Notifications are off") },
                        supportingContent = { Text("You will not see files, links or clips arriving from your PC. Tap to turn them on") },
                        modifier = Modifier.clickable {
                            ctx.startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, ctx.packageName))
                        },
                    )
                }
            }
            item { Text("Recent", color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(start = 16.dp, top = 8.dp)) }
            if (recent.isEmpty()) {
                item { Text("Nothing yet. Share a photo or file to $APP_NAME from any app.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(16.dp)) }
            }
            items(recent) { r ->
                ListItem(
                    headlineContent = { Text(r.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    supportingContent = { Text(r.subtitle) },
                    leadingContent = { KindBadge(r.kind) },
                    modifier = Modifier.clickable(enabled = r.uri != null) {
                        r.uri?.let {
                            // No app for this file type, or the file is gone.
                            runCatching { ctx.startActivity(Intent(Intent.ACTION_VIEW).setData(Uri.parse(it)).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)) }
                                .onFailure { Bridge.pairMessage.value = "No app can open ${r.title}" }
                        }
                    },
                )
            }
        }
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
