package dev.phonebridge.app

import android.content.BroadcastReceiver
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.PixelFormat
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.WindowManager

/**
 * Debug only: `am broadcast -n dev.phonebridge.app/.ClipProbeReceiver --es mode overlay|overlay_plain`.
 * Reads the clipboard through a 1x1 see-through overlay that takes focus for a moment, and logs
 * how long that took and what the read gave (lengths and kinds only, never the content).
 */
class ClipProbeReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        val mode = intent.getStringExtra("mode") ?: "overlay"
        val app = ctx.applicationContext
        val size = intent.getIntExtra("size", 1)
        val alphaArg = intent.getFloatExtra("alpha", 0f)
        val t0 = SystemClock.elapsedRealtime()
        val wm = app.getSystemService(WindowManager::class.java)
        val cm = app.getSystemService(ClipboardManager::class.java)
        var done = false
        val main = Handler(Looper.getMainLooper())
        // "overlay": keyboard-friendly flags; "overlay_plain": the flags other apps use without them.
        val keep = if (mode == "overlay") WindowManager.LayoutParams.FLAG_ALT_FOCUSABLE_IM else 0
        val view = object : View(app) {
            override fun onWindowFocusChanged(hasFocus: Boolean) {
                super.onWindowFocusChanged(hasFocus)
                if (!hasFocus || done) return
                done = true
                val at = SystemClock.elapsedRealtime() - t0
                val desc = cm.primaryClipDescription
                val ts = desc?.timestamp ?: 0L
                val clip = cm.primaryClip
                val item = clip?.takeIf { it.itemCount > 0 }?.getItemAt(0)
                Log.i(TAG, "PROBE mode=$mode size=$size alpha=$alphaArg focus_after_ms=$at read_ms=${SystemClock.elapsedRealtime() - t0} " +
                    "desc=${desc != null} age_s=${if (ts > 0) (System.currentTimeMillis() - ts) / 1000 else -1} " +
                    "text_len=${item?.text?.length ?: -1} uri=${item?.uri != null} mime=${desc?.getMimeType(0)}")
                runCatching { wm.removeView(this) }
            }
        }
        val lp = WindowManager.LayoutParams(
            size, size, WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or keep,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.START or Gravity.TOP
            alpha = alphaArg
            if (mode == "overlay") softInputMode = WindowManager.LayoutParams.SOFT_INPUT_STATE_UNCHANGED
        }
        runCatching { wm.addView(view, lp) }.onFailure { Log.i(TAG, "PROBE add failed: $it"); return }
        // Take focus: clear NOT_FOCUSABLE (the ALT flag stays where it was asked for).
        lp.flags = lp.flags and WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE.inv()
        runCatching { wm.updateViewLayout(view, lp) }
        main.postDelayed({
            if (!done) {
                done = true
                Log.i(TAG, "PROBE mode=$mode size=$size alpha=$alphaArg NO FOCUS after 2500 ms")
                runCatching { wm.removeView(view) }
            }
        }, 2500)
    }

    private companion object { const val TAG = "ClipProbe" }
}
