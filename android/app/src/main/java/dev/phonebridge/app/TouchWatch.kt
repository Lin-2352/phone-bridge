package dev.phonebridge.app

import android.content.Context
import android.graphics.PixelFormat
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager

/**
 * Knows when a finger touched the screen in another app (Android only says "a touch happened outside",
 * never where or what) so the clipboard is read once after the touching stops, not on a timer that
 * interrupts text selection. The window is 1x1, cannot be touched and never takes focus.
 */
object TouchWatch {
    private const val TAG = "PhoneBridge"
    /** Quiet time after the last touch before the clipboard is read: a Copy tap has finished by then. */
    const val QUIET_MS = 700L
    private val main = Handler(Looper.getMainLooper())
    private var view: View? = null
    private var quiet: Runnable? = null

    fun start(ctx: Context, onQuiet: () -> Unit) = main.post {
        val app = ctx.applicationContext
        if (view != null || !Settings.canDrawOverlays(app)) return@post
        val wm = app.getSystemService(WindowManager::class.java)
        val v = object : View(app) {
            override fun onTouchEvent(event: MotionEvent): Boolean {
                if (event.actionMasked == MotionEvent.ACTION_OUTSIDE) {
                    quiet?.let { main.removeCallbacks(it) }
                    quiet = Runnable { onQuiet() }.also { main.postDelayed(it, QUIET_MS) }
                }
                return false
            }
        }
        val lp = WindowManager.LayoutParams(
            1, 1, WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
                WindowManager.LayoutParams.FLAG_WATCH_OUTSIDE_TOUCH,
            PixelFormat.TRANSLUCENT,
        ).apply { gravity = Gravity.START or Gravity.TOP }
        if (runCatching { wm.addView(v, lp) }.isSuccess) view = v else Log.w(TAG, "TOUCH watch could not open its window")
    }

    fun stop(ctx: Context) = main.post {
        quiet?.let { main.removeCallbacks(it) }
        view?.let { v -> runCatching { ctx.applicationContext.getSystemService(WindowManager::class.java).removeView(v) } }
        view = null
    }
}
