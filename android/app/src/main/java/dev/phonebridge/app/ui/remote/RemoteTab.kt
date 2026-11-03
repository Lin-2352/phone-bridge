package dev.phonebridge.app.ui.remote

import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Scaffold
import dev.phonebridge.app.ui.components.EmptyState
import dev.phonebridge.app.ui.components.RelayTopBar
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
fun remoteState(running: Boolean, ready: Boolean, peerCount: Int?): RemoteState = when {
    !running -> RemoteState.Off
    !ready || peerCount == null -> RemoteState.Starting
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
    // null until read: the tab says Starting, never a flash of "Pair your PC first".
    val peers by produceState<Int?>(null, ready, linked) { value = if (ready) withContext(Dispatchers.IO) { Bridge.node?.peers()?.size ?: 0 } else null }
    val state = remoteState(running, ready, peers)
    Scaffold(topBar = { RelayTopBar("Remote", "Use this phone as your PC's touchpad and keyboard.") }) { pad ->
    Column(Modifier.fillMaxSize().padding(pad), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        when (state) {
            RemoteState.Ready -> TouchpadPad(Modifier.weight(1f).fillMaxSize())
            else -> Box(Modifier.weight(1f).fillMaxSize(), contentAlignment = Alignment.Center) {
                when (state) {
                    RemoteState.Off -> EmptyState("$APP_NAME is off", "Your PC cannot reach this phone.") {
                        Button(onClick = { setBridgeEnabled(ctx, true) }) { Text("Turn on") }
                    }
                    RemoteState.Starting -> EmptyState("Starting\u2026")
                    else -> EmptyState("Pair your PC first", "On your PC, open $APP_NAME and choose Pair new device, then tap Pair.") {
                        Button(onClick = { ctx.startActivity(Intent(ctx, ScanActivity::class.java)) }) { Text("Scan code") }
                    }
                }
            }
        }
    }
    }
}
