package dev.phonebridge.app.ui.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import dev.phonebridge.app.APP_NAME
import dev.phonebridge.app.ui.components.RelayRow

/**
 * The audio helper's state, read from a real answer while the row is on screen, its one-time setup,
 * and how to turn it off. What setup hands over is said before anything happens.
 */
@Composable
internal fun HelperRow() {
    val ctx = LocalContext.current
    var step by remember { mutableStateOf<dev.phonebridge.app.audio.HelperStep?>(null) }
    var dialog by remember { mutableStateOf(false) }
    androidx.compose.runtime.LaunchedEffect(Unit) {
        while (true) {
            step = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { dev.phonebridge.app.HelperStarter.step(ctx) }
            kotlinx.coroutines.delay(3000)
        }
    }
    val s = step
    RelayRow(
        title = "Phone audio helper",
        supporting = if (s == null) "Checking..." else dev.phonebridge.app.audio.helperRowText(s),
        onClick = { if (s != null && s != dev.phonebridge.app.audio.HelperStep.UNSUPPORTED) dialog = true },
    )
    if (!dialog || s == null) return
    val on = s != dev.phonebridge.app.audio.HelperStep.NOT_SET_UP && s != dev.phonebridge.app.audio.HelperStep.NEEDS_SETUP_AGAIN
    androidx.compose.material3.AlertDialog(
        onDismissRequest = { dialog = false },
        title = { androidx.compose.material3.Text(if (on) "Turn off the helper?" else "Set up the helper?") },
        text = {
            androidx.compose.material3.Text(
                if (on) {
                    "$APP_NAME stops it and gives up its access. Sharing goes back to Android's prompt. To clear the last trace, remove $APP_NAME from Paired devices in Wireless debugging."
                } else {
                    "Once only: turn on Wireless debugging, tap Pair device with pairing code, and type the code into $APP_NAME's notification. Tick Always allow on this network if asked.\n\n" +
                        "$APP_NAME then holds shell access to this phone, kept encrypted and used only to start the helper. It can record everything this phone plays, including apps that block recording, and Android shows no capture icon: $APP_NAME's notification with Stop is the only sign."
                },
            )
        },
        confirmButton = {
            androidx.compose.material3.TextButton(onClick = {
                dialog = false
                if (on) dev.phonebridge.app.HelperStarter.turnOff(ctx) else dev.phonebridge.app.HelperStarter.beginSetup(ctx)
            }) { androidx.compose.material3.Text(if (on) "Turn off" else "Set up") }
        },
        dismissButton = {
            androidx.compose.material3.TextButton(onClick = { dialog = false }) { androidx.compose.material3.Text("Cancel") }
        },
    )
}
