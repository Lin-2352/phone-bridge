package dev.phonebridge.app.ui.remote

import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import dev.phonebridge.app.APP_NAME
import dev.phonebridge.app.Bridge
import dev.phonebridge.app.ScanActivity
import dev.phonebridge.app.setBridgeEnabled
import androidx.compose.runtime.produceState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

enum class RemoteState { Off, Starting, NeedsPairing, Ready }

/** What the tab shows. "Pair" only once the node is up and really has no peer, never from a cold start. */
fun remoteState(running: Boolean, ready: Boolean, peerCount: Int): RemoteState = when {
    !running -> RemoteState.Off
    !ready -> RemoteState.Starting
    peerCount == 0 -> RemoteState.NeedsPairing
    else -> RemoteState.Ready
}

/** Remote tab: header, then the touchpad filling the rest; the Phase E button deck goes below it. */
@Composable
fun RemoteTab() {
    val ctx = LocalContext.current
    val running = Bridge.running.value
    val ready = Bridge.nodeReady.value
    val linked = Bridge.linked.value
    val peers by produceState(0, ready, linked) { value = if (ready) withContext(Dispatchers.IO) { Bridge.node?.peers()?.size ?: 0 } else 0 }
    val state = remoteState(running, ready, peers)
    Column(Modifier.fillMaxSize().safeDrawingPadding(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp)) {
            Text("Remote", style = MaterialTheme.typography.headlineSmall)
            Text(
                "Use this phone as your PC's touchpad and keyboard.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        when (state) {
            RemoteState.Ready -> TouchpadPad(Modifier.weight(1f).fillMaxSize())
            else -> Box(Modifier.weight(1f).fillMaxSize(), contentAlignment = Alignment.Center) {
                Column(Modifier.padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    when (state) {
                        RemoteState.Off -> {
                            Text("$APP_NAME is off", style = MaterialTheme.typography.titleMedium)
                            Text("Your PC cannot reach this phone.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Button(onClick = { setBridgeEnabled(ctx, true) }) { Text("Turn on") }
                        }
                        RemoteState.Starting -> Text("Starting\u2026", style = MaterialTheme.typography.titleMedium)
                        else -> {
                            Text("Pair your PC first", style = MaterialTheme.typography.titleMedium)
                            Text(
                                "On your PC, open $APP_NAME and choose Pair new device, then tap Pair.",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Button(onClick = { ctx.startActivity(Intent(ctx, ScanActivity::class.java)) }) { Text("Pair") }
                        }
                    }
                }
            }
        }
    }
}
