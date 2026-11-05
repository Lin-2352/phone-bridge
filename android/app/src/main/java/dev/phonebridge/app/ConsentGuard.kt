package dev.phonebridge.app

import android.app.Activity
import android.os.Build
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner

/**
 * Relay's own prompts are on screen (consent, pairing code, Android's capture prompt): the PC's
 * remote input pauses meanwhile, so a PC that already has Control cannot tap "Allow" itself.
 * A prompt that never reports back stops counting after [EXPIRY_MS].
 */
object ConsentGuard {
    const val EXPIRY_MS = 120_000L
    private val open = HashMap<String, Long>()

    /** Called when the first prompt opens: ControlService releases held keys. */
    @Volatile var onFirstBegin: (() -> Unit)? = null

    fun begin(id: String, nowMs: Long) {
        val first = synchronized(this) {
            val wasActive = activeLocked(nowMs)
            open[id] = nowMs
            !wasActive
        }
        if (first) onFirstBegin?.invoke()
    }

    @Synchronized fun end(id: String) {
        open.remove(id)
    }

    @Synchronized fun active(nowMs: Long): Boolean = activeLocked(nowMs)

    private fun activeLocked(nowMs: Long): Boolean {
        open.values.removeAll { nowMs - it >= EXPIRY_MS }
        return open.isNotEmpty()
    }

    /** Begins when the activity is started, ends when it stops. */
    fun observer(id: String) = object : DefaultLifecycleObserver {
        override fun onStart(owner: LifecycleOwner) = begin(id, SystemClock.elapsedRealtime())
        override fun onStop(owner: LifecycleOwner) = end(id)
    }

    @Synchronized internal fun clearForTest() = open.clear()
}

/** A touch that went through, or next to, another app's window is not a decision. */
fun isObscured(motionFlags: Int): Boolean =
    motionFlags and (MotionEvent.FLAG_WINDOW_IS_OBSCURED or MotionEvent.FLAG_WINDOW_IS_PARTIALLY_OBSCURED) != 0

/** Protections for a window that asks for consent. */
object SecureWindow {
    fun apply(activity: Activity) {
        // Other apps' overlays are hidden while this window shows (needs HIDE_OVERLAY_WINDOWS).
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) activity.window.setHideOverlayWindows(true)
        activity.window.decorView.filterTouchesWhenObscured = true
        // Only accessibility tools (screen readers) may read or act on it.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            activity.window.decorView.setAccessibilityDataSensitive(View.ACCESSIBILITY_DATA_SENSITIVE_YES)
        }
    }
}
