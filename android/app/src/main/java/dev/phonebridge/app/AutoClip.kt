package dev.phonebridge.app

import android.app.Activity
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.provider.Settings
import android.util.Log

/**
 * Whether a clipboard change should wake [AutoClipActivity]: Share clipboard is on, the phone is awake and Relay may
 * open a window from the background ("Display over other apps"), and the last wake was not a moment ago.
 */
fun shouldAutoRead(syncOn: Boolean, interactive: Boolean, overlay: Boolean, lastWakeMs: Long, nowMs: Long): Boolean =
    syncOn && interactive && overlay && (lastWakeMs == 0L || nowMs - lastWakeMs >= AUTO_READ_GAP_MS)

const val AUTO_READ_GAP_MS = 800L

/**
 * Android only lets the focused app read the clipboard, but it does tell every app when the clipboard changed. With
 * "Display over other apps" allowed, Relay may then open a see-through window for a moment: it gets focus, reads
 * the copy, sends it (the same path as opening Relay) and closes. Without that permission nothing changes: copies
 * are sent when Relay is opened or from its tile.
 */
class AutoClipWatcher(private val ctx: Context) : ClipboardManager.OnPrimaryClipChangedListener {
    private val cm = ctx.getSystemService(ClipboardManager::class.java)
    private var lastWake = 0L

    fun start() = cm.addPrimaryClipChangedListener(this)
    fun stop() = cm.removePrimaryClipChangedListener(this)

    override fun onPrimaryClipChanged() {
        val now = android.os.SystemClock.elapsedRealtime()
        val power = ctx.getSystemService(android.os.PowerManager::class.java)
        if (!shouldAutoRead(Prefs(ctx).clipboardSync, power.isInteractive, Settings.canDrawOverlays(ctx), lastWake, now)) return
        // A copy that arrived from the PC was remembered before it was written: nothing to send, no window.
        lastWake = now
        runCatching {
            ctx.startActivity(
                Intent(ctx, AutoClipActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION or Intent.FLAG_ACTIVITY_NO_HISTORY),
            )
        }.onFailure { Log.w("PhoneBridge", "AUTOCLIP could not open its window: $it") }
    }
}

/** See-through and short: takes focus, reads the clipboard through the normal send path, closes. */
class AutoClipActivity : Activity() {
    private var done = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        overridePendingTransition(0, 0)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || done) return
        done = true
        Log.i("PhoneBridge", "AUTOCLIP read")
        shareClipboardIfChanged(applicationContext, Prefs(this))
        finish()
        overridePendingTransition(0, 0)
    }
}
