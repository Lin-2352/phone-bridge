package dev.phonebridge.app

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.graphics.PixelFormat
import android.media.AudioManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.net.wifi.WifiManager
import android.view.Choreographer
import android.view.Gravity
import android.view.ViewGroup
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.Toast
import uniffi.pb_ffi.InputListener
import kotlin.math.abs

private const val TAG = "PhoneBridge"

/**
 * PC -> phone control. While the user keeps this accessibility service on, a
 * paired PC can drive the phone: an on-screen cursor follows the PC mouse,
 * clicks become taps and swipes, the wheel scrolls, and typing goes into the
 * focused text field. Nothing happens unless the PC starts a session.
 *
 * Smooth pointer: the cursor lives in one full-screen overlay created for
 * the session and moves by translation (render thread only, no window
 * relayout), once per display frame (120 Hz on this phone), easing towards
 * the true position so moves that arrive in bursts still look continuous.
 * Taps always use the true position. A Wi-Fi lock is held during control so
 * power save does not batch the pointer packets.
 */
class ControlService : AccessibilityService(), InputListener {
    private val main = Handler(Looper.getMainLooper())
    private var overlay: FrameLayout? = null
    private var cursor: ImageView? = null
    /** True pointer position (taps go here) and the drawn one (eases towards it). */
    private var x = 0f
    private var y = 0f
    private var shownX = 0f
    private var shownY = 0f
    private var frameQueued = false
    private val frame = Choreographer.FrameCallback { onFrame() }
    private var wifiLocks: List<WifiManager.WifiLock> = emptyList()
    private val stats = MoveStats()
    private var w = 1080
    private var h = 2400
    private var pressX = 0f
    private var pressY = 0f
    private var pressAt = 0L
    private var ctrl = false
    private var highSurrogate: Char? = null

    override fun onServiceConnected() {
        instance = this
        Bridge.node?.setInputListener(this)
        if (Build.VERSION.SDK_INT >= 30) {
            val b = getSystemService(WindowManager::class.java).currentWindowMetrics.bounds
            w = b.width()
            h = b.height()
        } else {
            val m = android.util.DisplayMetrics()
            @Suppress("DEPRECATION")
            getSystemService(WindowManager::class.java).defaultDisplay.getRealMetrics(m)
            w = m.widthPixels
            h = m.heightPixels
        }
        Log.i(TAG, "CONTROL service on (${w}x$h)")
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        Bridge.node?.setInputListener(null)
        hideCursor()
        holdWifi(false)
        Bridge.controlledBy.value = null
        super.onDestroy()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}
    override fun onInterrupt() {}

    // InputListener: called on a Rust thread; all work happens on the main thread.

    override fun onControl(active: Boolean, peerName: String) {
        main.post {
            if (active) {
                x = w / 2f
                y = h / 2f
                showCursor()
                holdWifi(true)
                stats.reset()
                Bridge.controlledBy.value = peerName
                Toast.makeText(this, "$peerName is controlling this phone", Toast.LENGTH_SHORT).show()
            } else {
                hideCursor()
                holdWifi(false)
                Bridge.controlledBy.value = null
                Toast.makeText(this, "$peerName stopped controlling this phone", Toast.LENGTH_SHORT).show()
            }
            PbService.refresh(this)
            Log.i(TAG, "CONTROL ${if (active) "started" else "ended"} by $peerName")
        }
    }

    private val moveLock = Any()
    private var pendingDx = 0
    private var pendingDy = 0
    private var moveQueued = false

    override fun onInput(kind: UByte, a: Int, b: Int, pressed: Boolean) {
        if (kind.toInt() == 0) {
            // Pointer moves arrive up to 1000/s; merge them and touch the
            // window manager at most once per main-loop turn.
            synchronized(moveLock) {
                stats.arrived(SystemClock.uptimeMillis())
                pendingDx += a
                pendingDy += b
                if (moveQueued) return
                moveQueued = true
            }
            main.post {
                val (dx, dy) = synchronized(moveLock) {
                    moveQueued = false
                    (pendingDx to pendingDy).also { pendingDx = 0; pendingDy = 0 }
                }
                move(dx, dy)
            }
            return
        }
        main.post {
            when (kind.toInt()) {
                1 -> button(a, pressed)
                2 -> wheel(a, b)
                3 -> key(a, pressed)
                4 -> char(a)
            }
        }
    }

    private fun showCursor() {
        if (overlay != null) return
        val size = (22 * resources.displayMetrics.density).toInt()
        val c = ImageView(this).apply {
            setImageResource(R.drawable.ic_cursor)
            layoutParams = FrameLayout.LayoutParams(size, size)
        }
        val root = FrameLayout(this).apply { addView(c) }
        val p = WindowManager.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT,
            WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or
                WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            // Screen coordinates, including the cutout area, so the drawn
            // cursor sits exactly where taps land.
            layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
        }
        getSystemService(WindowManager::class.java).addView(root, p)
        overlay = root
        cursor = c
        shownX = x
        shownY = y
        c.translationX = x
        c.translationY = y
    }

    private fun hideCursor() {
        if (frameQueued) Choreographer.getInstance().removeFrameCallback(frame)
        frameQueued = false
        overlay?.let { runCatching { getSystemService(WindowManager::class.java).removeView(it) } }
        overlay = null
        cursor = null
    }

    private fun move(dx: Int, dy: Int) {
        x = (x + dx * 1.4f).coerceIn(0f, w - 1f)
        y = (y + dy * 1.4f).coerceIn(0f, h - 1f)
        if (cursor != null && !frameQueued) {
            frameQueued = true
            Choreographer.getInstance().postFrameCallback(frame)
        }
    }

    /** One display frame: ease the drawn cursor towards the true position. */
    private fun onFrame() {
        frameQueued = false
        val c = cursor ?: return
        val ex = x - shownX
        val ey = y - shownY
        // Most of the way each frame (about 2 frames to catch up at 120 Hz).
        if (abs(ex) < 0.75f && abs(ey) < 0.75f) {
            shownX = x
            shownY = y
        } else {
            shownX += ex * 0.6f
            shownY += ey * 0.6f
        }
        c.translationX = shownX
        c.translationY = shownY
        stats.frame(SystemClock.uptimeMillis())
        if (shownX != x || shownY != y) {
            frameQueued = true
            Choreographer.getInstance().postFrameCallback(frame)
        }
    }

    /** Keep Wi-Fi out of power save while controlled (pointer packets in time). */
    private fun holdWifi(on: Boolean) {
        wifiLocks.forEach { runCatching { if (it.isHeld) it.release() } }
        wifiLocks = emptyList()
        if (!on) return
        val wm = applicationContext.getSystemService(WifiManager::class.java) ?: return
        @Suppress("DEPRECATION")
        wifiLocks = listOf(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, WifiManager.WIFI_MODE_FULL_HIGH_PERF).mapNotNull { mode ->
            runCatching { wm.createWifiLock(mode, "PhoneBridge:control").apply { setReferenceCounted(false); acquire() } }.getOrNull()
        }
    }

    private fun stroke(path: Path, duration: Long) {
        val g = GestureDescription.Builder()
            .addStroke(GestureDescription.StrokeDescription(path, 0, duration.coerceIn(1, 3000)))
            .build()
        dispatchGesture(g, null, null)
    }

    private fun button(b: Int, pressed: Boolean) {
        when (b) {
            0 -> if (pressed) {
                pressX = x
                pressY = y
                pressAt = SystemClock.uptimeMillis()
            } else {
                val held = SystemClock.uptimeMillis() - pressAt
                val path = Path().apply { moveTo(pressX, pressY) }
                if (abs(x - pressX) < 12 && abs(y - pressY) < 12) {
                    // Click = tap; a long press stays a long press.
                    stroke(path, if (held > 500) held else 40)
                } else {
                    path.lineTo(x, y)
                    stroke(path, held.coerceIn(80, 1500))
                }
            }
            1 -> if (pressed) performGlobalAction(GLOBAL_ACTION_BACK)
            2 -> if (pressed) performGlobalAction(GLOBAL_ACTION_HOME)
            3 -> if (pressed) performGlobalAction(GLOBAL_ACTION_BACK)
            4 -> if (pressed) performGlobalAction(GLOBAL_ACTION_RECENTS)
        }
    }

    /** One wheel notch (120) scrolls about a fifth of the screen. */
    private fun wheel(dx: Int, dy: Int) {
        val dist = h / 5f * (dy / 120f)
        val distX = w / 5f * (dx / 120f)
        val path = Path().apply {
            moveTo(x, y)
            // Wheel up shows content above: the finger moves down.
            lineTo((x - distX).coerceIn(0f, w - 1f), (y + dist).coerceIn(0f, h - 1f))
        }
        stroke(path, 120)
    }

    private fun focused(): AccessibilityNodeInfo? =
        rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.takeIf { it.isEditable }

    private fun currentText(n: AccessibilityNodeInfo): String =
        if (Build.VERSION.SDK_INT >= 26 && n.isShowingHintText) "" else n.text?.toString() ?: ""

    private fun setText(n: AccessibilityNodeInfo, text: String, cursorAt: Int) {
        n.performAction(
            AccessibilityNodeInfo.ACTION_SET_TEXT,
            Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text) },
        )
        n.performAction(
            AccessibilityNodeInfo.ACTION_SET_SELECTION,
            Bundle().apply {
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, cursorAt)
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, cursorAt)
            },
        )
    }

    private fun insert(s: String) {
        val n = focused() ?: return
        val t = currentText(n)
        var a = n.textSelectionStart
        var b = n.textSelectionEnd
        if (a < 0 || b < 0) {
            a = t.length
            b = t.length
        }
        val (lo, hi) = minOf(a, b).coerceAtMost(t.length) to maxOf(a, b).coerceAtMost(t.length)
        setText(n, t.replaceRange(lo, hi, s), lo + s.length)
    }

    private fun char(unit: Int) {
        val c = unit.toChar()
        if (c.isHighSurrogate()) {
            highSurrogate = c
            return
        }
        val s = highSurrogate?.let { "$it$c" } ?: c.toString()
        highSurrogate = null
        insert(s)
    }

    private fun key(code: Int, pressed: Boolean) {
        if (code == 0x1D || code == 0xE01D) {
            ctrl = pressed
            return
        }
        if (!pressed) return
        val n = focused()
        when {
            ctrl && code == 0x1E -> n?.let { // Ctrl+A
                val len = currentText(it).length
                it.performAction(
                    AccessibilityNodeInfo.ACTION_SET_SELECTION,
                    Bundle().apply {
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, 0)
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, len)
                    },
                )
            }
            ctrl && code == 0x2E -> n?.performAction(AccessibilityNodeInfo.ACTION_COPY)
            ctrl && code == 0x2F -> n?.performAction(AccessibilityNodeInfo.ACTION_PASTE)
            ctrl && code == 0x2D -> n?.performAction(AccessibilityNodeInfo.ACTION_CUT)
            code == 0x0E -> n?.let { // Backspace
                val t = currentText(it)
                var a = it.textSelectionStart
                var b = it.textSelectionEnd
                if (a < 0 || b < 0) {
                    a = t.length
                    b = t.length
                }
                val lo = minOf(a, b).coerceAtMost(t.length)
                val hi = maxOf(a, b).coerceAtMost(t.length)
                if (hi > lo) {
                    setText(it, t.removeRange(lo, hi), lo)
                } else if (lo > 0) {
                    val from = if (lo >= 2 && t[lo - 1].isLowSurrogate()) lo - 2 else lo - 1
                    setText(it, t.removeRange(from, lo), from)
                }
            }
            code == 0x1C -> n?.let { // Enter
                if (Build.VERSION.SDK_INT >= 30) {
                    it.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
                } else {
                    insert("\n")
                }
            }
            code == 0xE04B || code == 0xE04D -> n?.performAction(
                if (code == 0xE04B) AccessibilityNodeInfo.ACTION_PREVIOUS_AT_MOVEMENT_GRANULARITY
                else AccessibilityNodeInfo.ACTION_NEXT_AT_MOVEMENT_GRANULARITY,
                Bundle().apply {
                    putInt(
                        AccessibilityNodeInfo.ACTION_ARGUMENT_MOVEMENT_GRANULARITY_INT,
                        AccessibilityNodeInfo.MOVEMENT_GRANULARITY_CHARACTER,
                    )
                },
            )
            code == 0x01 -> performGlobalAction(GLOBAL_ACTION_BACK) // Esc
            code == 0xE05B || code == 0xE05C -> performGlobalAction(GLOBAL_ACTION_HOME) // Win
            code == 0xE030 || code == 0xE02E -> getSystemService(AudioManager::class.java).adjustStreamVolume(
                AudioManager.STREAM_MUSIC,
                if (code == 0xE030) AudioManager.ADJUST_RAISE else AudioManager.ADJUST_LOWER,
                AudioManager.FLAG_SHOW_UI,
            )
        }
    }

    /**
     * Pointer timing during a session, logged every 2 s: move packets per
     * second, gaps between them (p50 / p95 / max, ms) and frames drawn per
     * second. Shows whether jank comes from the network or from drawing.
     */
    private class MoveStats {
        private val gaps = IntArray(1024)
        private var n = 0
        private var last = 0L
        private var frames = 0
        private var since = 0L

        @Synchronized fun reset() {
            n = 0; last = 0L; frames = 0; since = SystemClock.uptimeMillis()
        }

        @Synchronized fun arrived(now: Long) {
            if (last != 0L && n < gaps.size) gaps[n++] = (now - last).toInt()
            last = now
            report(now)
        }

        @Synchronized fun frame(now: Long) {
            frames++
            report(now)
        }

        private fun report(now: Long) {
            val span = now - since
            if (span < 2000) return
            if (n > 0) {
                val g = gaps.copyOf(n).sorted()
                Log.i(
                    TAG,
                    "CONTROL timing moves/s=${n * 1000 / span} gap_ms p50=${g[n / 2]} p95=${g[(n * 95) / 100]} max=${g[n - 1]} frames/s=${frames * 1000 / span}",
                )
            }
            n = 0; frames = 0; since = now
        }
    }

    companion object {
        /** The running service, so a node started later can register it. */
        @Volatile var instance: ControlService? = null
    }
}
