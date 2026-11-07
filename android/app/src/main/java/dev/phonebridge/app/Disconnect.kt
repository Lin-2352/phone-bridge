package dev.phonebridge.app

import android.content.Context
import android.content.Intent
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/** The preference keys that hold an "always allow" answer: control, screen and camera per PC, and phone audio. */
fun alwaysAllowKeysToClear(): List<String> = listOf(
    "allow_control_pcs",
    "allow_screen_pcs",
    "allow_camera_pcs",
    "allow_audio_pcs",
    "audio_skip_confirm_phone_to_pc",
    // Pending global answers (set while Relay was off) that the next start would fold into every PC.
    "allow_control",
    "allow_screen",
)

/** True for a key that holds an "always allow" answer: one of the fixed keys, or a PC's automatic file accept. */
fun isAlwaysAllowKey(key: String): Boolean = key in alwaysAllowKeysToClear() || key.endsWith("/auto_accept")

/** The preferences after "Disconnect everything": every always-allow answer removed, and Relay off until resumed. */
fun afterDisconnect(all: Map<String, Any?>): Map<String, Any?> =
    all.filterKeys { !isAlwaysAllowKey(it) } + mapOf("enabled" to false, "auto_accept" to false)

/** The preferences after the person resumes Relay: it is on again; the removed answers are not restored. */
fun afterResume(all: Map<String, Any?>): Map<String, Any?> = all + ("enabled" to true)

/**
 * "Disconnect everything" (tile, notification action, Home): closes every connection, removes every always-allow
 * answer, and stops Relay until the person turns it on again. It only takes access away, so it asks nothing.
 */
fun disconnectEverything(ctx: Context) {
    val prefs = Prefs(ctx)
    prefs.removeAlwaysAllow()
    prefs.enabled = false
    // Close every connection first (the node is only there while the service runs), then stop the service.
    val node = Bridge.node
    CoroutineScope(Dispatchers.IO).launch {
        node?.disconnectAll()
        ctx.startService(Intent(ctx, PbService::class.java).setAction(PbService.ACTION_STOP))
    }
}
