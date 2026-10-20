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
import android.view.KeyEvent
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
 *
 * Typing goes through the service's own input method (Android 13+), like a
 * keyboard: text is committed at the cursor and keys are real key events, so
 * fast typing keeps its order and a field's placeholder is never taken as
 * text. Older versions fall back to setting the field's text.
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
    private var ctrl = false
    private var highSurrogate: Char? = null

    override fun onServiceConnected() {
        instance = this
        Bridge.node?.setInputListener(this)
        readBounds()
        Log.i(TAG, "CONTROL service on (${w}x$h)")
        // Android may have stopped Phone Bridge (low memory); try to bring it
        // back. If refused, the "not running" notice tells the user.
        ensureRunning(this, "control")
    }

    /** Screen size now (it changes with rotation). */
    private fun readBounds() {
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
    }

    override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {
        super.onConfigurationChanged(newConfig)
        readBounds()
        x = x.coerceIn(0f, w - 1f)
        y = y.coerceIn(0f, h - 1f)
    }

    override fun onDestroy() {
        if (instance === this) {
            instance = null
            Bridge.node?.setInputListener(null)
            // Turned off mid-session: end it, or the PC keeps "controlling" nothing.
            if (Bridge.controlledBy.value != null) Bridge.node?.stopControl()
        }
        hideCursor()
        holdWifi(false)
        Bridge.controlledBy.value = null
        PbService.refresh(this)
        super.onDestroy()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}
    override fun onInterrupt() {}

    // InputListener: called on a Rust thread; all work happens on the main thread.

    /**
     * A screen or control session started or ended: no key is held any more
     * (a key-up lost with the old session must not leave Ctrl down, which
     * would turn the next "a" into select-all).
     */
    fun resetKeys() {
        main.post {
            ctrl = false
            if (finger.down) finger.release()
        }
    }

    override fun onControl(active: Boolean, peerName: String) {
        main.post {
            ctrl = false
            if (active) {
                readBounds()
                x = w / 2f
                y = h / 2f
                showCursor()
                holdWifi(true)
                stats.reset()
                Bridge.controlledBy.value = peerName
                Toast.makeText(this, "$peerName is controlling this phone", Toast.LENGTH_SHORT).show()
            } else {
                stopWheel()
                finger.release()
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

    /**
     * The phone-screen window: a finger at an absolute spot (x, y in
     * 0..65535 across the screen). Hover only places the pointer (the next
     * wheel scrolls there). Main thread.
     */
    fun touchAt(phase: Int, ax: Int, ay: Int) {
        x = ax / 65535f * (w - 1)
        y = ay / 65535f * (h - 1)
        when (phase) {
            1 -> {
                stopWheel()
                finger.press(x, y)
            }
            2 -> if (finger.down && !wheeling) finger.moveTo(x, y)
            3 -> finger.release()
        }
        if (cursor != null && !frameQueued) {
            frameQueued = true
            Choreographer.getInstance().postFrameCallback(frame)
        }
    }

    /** Two fingers pinching out (zoom in) or in, around (x, y). Main thread. */
    fun pinchAt(out: Boolean, ax: Int, ay: Int) {
        val cx = ax / 65535f * (w - 1)
        val cy = ay / 65535f * (h - 1)
        val near = 60f * resources.displayMetrics.density / 2.75f
        val far = 210f * resources.displayMetrics.density / 2.75f
        val (from, to) = if (out) near to far else far to near
        fun stroke(sign: Int) = GestureDescription.StrokeDescription(
            Path().apply {
                moveTo((cx + sign * from).coerceIn(0f, w - 1f), cy)
                lineTo((cx + sign * to).coerceIn(0f, w - 1f), cy)
            },
            0, 300,
        )
        dispatchGesture(GestureDescription.Builder().addStroke(stroke(-1)).addStroke(stroke(1)).build(), null, null)
    }

    /** 1×1 window holding the screen awake and, when dimming, the backlight down. */
    private var dimView: android.view.View? = null
    private val screenOff = object : android.content.BroadcastReceiver() {
        override fun onReceive(context: android.content.Context, intent: android.content.Intent) {
            // The power button: brightness comes back, but the phone keeps
            // awake for the mirror (the overlay stays) and the PC is told.
            setDim(false)
            ScreenShare.dimEnded()
        }
    }

    /** While mirroring: keep the phone awake; `on` also dims the backlight. Main thread. */
    fun setDim(on: Boolean) {
        val wm = getSystemService(WindowManager::class.java)
        val p = (dimView?.layoutParams as? WindowManager.LayoutParams) ?: WindowManager.LayoutParams(
            1, 1,
            WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON,
            PixelFormat.TRANSLUCENT,
        ).apply { gravity = Gravity.TOP or Gravity.START }
        p.screenBrightness = if (on) 0f else WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE
        val v = dimView
        if (v == null) {
            val nv = android.view.View(this)
            runCatching { wm.addView(nv, p) }.onSuccess {
                dimView = nv
                registerReceiver(screenOff, android.content.IntentFilter(android.content.Intent.ACTION_SCREEN_OFF))
            }
        } else {
            runCatching { wm.updateViewLayout(v, p) }
        }
        Log.i(TAG, "SCREEN dim=$on")
    }

    /** Mirroring ended or the power button was pressed. */
    fun endDim() {
        val v = dimView ?: return
        dimView = null
        runCatching { getSystemService(WindowManager::class.java).removeView(v) }
        runCatching { unregisterReceiver(screenOff) }
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
        if (finger.down && !wheeling) finger.moveTo(x, y)
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

    /**
     * One synthetic finger, streamed as continued strokes (API 26+): each
     * segment goes out only when the previous one has finished, so segments
     * never cancel each other, and each lasts as long as the hand took, so
     * the app sees real velocity (a quick drag flings like a finger). While
     * the pointer rests nothing is sent and the finger stays down, so
     * press-and-hold is a long press. Main thread only.
     */
    private inner class Finger {
        private var stroke: GestureDescription.StrokeDescription? = null
        private var inFlight = false
        var down = false
            private set
        private var lifting = false
        private var lastX = 0f
        private var lastY = 0f
        private var lastSentAt = 0L
        private val points = ArrayList<android.graphics.PointF>()
        private var generation = 0

        fun press(px: Float, py: Float) {
            reset()
            down = true
            lastX = px
            lastY = py
            lastSentAt = SystemClock.uptimeMillis()
            // A tiny first segment puts the finger down.
            send(Path().apply { moveTo(px, py); lineTo(px + 0.5f, py) }, 10, true)
            lastX = px + 0.5f
        }

        fun moveTo(px: Float, py: Float) {
            if (!down || lifting) return
            points.add(android.graphics.PointF(px, py))
            pump()
        }

        fun release() {
            if (!down) return
            lifting = true
            pump()
        }

        private fun pump() {
            if (inFlight || !down) return
            if (points.isEmpty() && !lifting) return
            val path = Path().apply {
                moveTo(lastX, lastY)
                if (points.isEmpty()) {
                    lineTo(lastX + 0.5f, lastY) // a path needs some length
                    lastX += 0.5f
                } else {
                    points.forEach { lineTo(it.x, it.y) }
                    lastX = points.last().x
                    lastY = points.last().y
                }
            }
            points.clear()
            val now = SystemClock.uptimeMillis()
            val dur = (now - lastSentAt).coerceIn(8, 50)
            lastSentAt = now
            val lift = lifting
            send(path, dur, !lift)
            if (lift) {
                down = false
                lifting = false
            }
        }

        private fun send(path: Path, dur: Long, willContinue: Boolean) {
            val prev = stroke
            val next = prev?.continueStroke(path, 0, dur, willContinue)
                ?: GestureDescription.StrokeDescription(path, 0, dur, willContinue)
            stroke = if (willContinue) next else null
            inFlight = true
            stats.segment()
            val gen = generation
            val sent = dispatchGesture(
                GestureDescription.Builder().addStroke(next).build(),
                object : GestureResultCallback() {
                    override fun onCompleted(gestureDescription: GestureDescription?) {
                        if (gen != generation) return
                        inFlight = false
                        pump()
                    }

                    // The system interrupted it (another gesture, a touch):
                    // start fresh, never leave a finger "down".
                    override fun onCancelled(gestureDescription: GestureDescription?) {
                        if (gen == generation) reset()
                    }
                },
                main,
            )
            if (!sent) reset()
        }

        fun reset() {
            generation++
            stroke = null
            inFlight = false
            down = false
            lifting = false
            points.clear()
        }
    }

    private val finger = Finger()

    /** Wheel scrolling streamed through the same finger, eased per frame. */
    private var wheeling = false
    private var wheelX = 0f
    private var wheelY = 0f
    private var wheelToX = 0f
    private var wheelToY = 0f
    private var wheelLastNotch = 0L
    private val wheelFrame = Choreographer.FrameCallback { onWheelFrame() }

    private fun button(b: Int, pressed: Boolean) {
        when (b) {
            0 -> if (pressed) {
                stopWheel()
                finger.press(x, y)
            } else {
                finger.release()
            }
            1 -> if (pressed) performGlobalAction(GLOBAL_ACTION_BACK)
            2 -> if (pressed) performGlobalAction(GLOBAL_ACTION_HOME)
            3 -> if (pressed) performGlobalAction(GLOBAL_ACTION_BACK)
            4 -> if (pressed) performGlobalAction(GLOBAL_ACTION_RECENTS)
        }
    }

    /**
     * One wheel notch (120) scrolls about a fifth of the screen. Notches that
     * arrive during a scroll extend it instead of starting a new gesture.
     */
    private fun wheel(dx: Int, dy: Int) {
        if (finger.down && !wheeling) return // dragging with the button held
        if (!wheeling) {
            wheeling = true
            wheelX = x
            wheelY = y
            wheelToX = x
            wheelToY = y
            finger.press(x, y)
            Choreographer.getInstance().postFrameCallback(wheelFrame)
        }
        // Wheel up shows content above: the finger moves down.
        wheelToY = (wheelToY + h / 5f * (dy / 120f)).coerceIn(1f, h - 2f)
        wheelToX = (wheelToX - w / 5f * (dx / 120f)).coerceIn(1f, w - 2f)
        wheelLastNotch = SystemClock.uptimeMillis()
    }

    private fun onWheelFrame() {
        if (!wheeling) return
        val ex = wheelToX - wheelX
        val ey = wheelToY - wheelY
        if (abs(ex) < 1f && abs(ey) < 1f) {
            // Arrived; lift once no new notch came for a moment.
            if (SystemClock.uptimeMillis() - wheelLastNotch > 60) {
                stopWheel()
                return
            }
        } else {
            // Ease: most of the remaining distance each frame (about 120 ms).
            wheelX += ex * 0.3f
            wheelY += ey * 0.3f
            finger.moveTo(wheelX, wheelY)
        }
        Choreographer.getInstance().postFrameCallback(wheelFrame)
    }

    private fun stopWheel() {
        if (!wheeling) return
        wheeling = false
        Choreographer.getInstance().removeFrameCallback(wheelFrame)
        finger.release()
    }

    private fun focused(): AccessibilityNodeInfo? =
        rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
            ?.takeIf { it.isEditable }
            // The node is a snapshot: re-read it, or edits build on stale text.
            ?.also { it.refresh() }

    private fun currentText(n: AccessibilityNodeInfo): String {
        if (n.isShowingHintText) return ""
        val t = n.text?.toString() ?: return ""
        // Some apps report their placeholder as the text of an empty field.
        return if (n.hintText != null && t == n.hintText.toString()) "" else t
    }

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

    /** Typed text not yet committed (characters of one main-loop turn). */
    private val typed = StringBuilder()
    private var typedQueued = false

    /** The focused field's input connection, if any. */
    @androidx.annotation.RequiresApi(33)
    private fun conn(): android.accessibilityservice.InputMethod.AccessibilityInputConnection? =
        inputMethod?.currentInputConnection

    private fun char(unit: Int) {
        val c = unit.toChar()
        if (c.isHighSurrogate()) {
            highSurrogate = c
            return
        }
        typed.append(highSurrogate ?: "").append(c)
        highSurrogate = null
        if (!typedQueued) {
            typedQueued = true
            main.post { flushTyped() }
        }
    }

    /** Commit what was typed, in order; before any key so order is kept. */
    private fun flushTyped() {
        typedQueued = false
        if (typed.isEmpty()) return
        val s = typed.toString()
        typed.setLength(0)
        if (Build.VERSION.SDK_INT >= 33) {
            val c = conn()
            if (c != null) {
                c.commitText(s, 1, null)
                return
            }
        }
        insert(s)
    }

    /** Press and release one Android key in the focused field. */
    @androidx.annotation.RequiresApi(33)
    private fun press(c: android.accessibilityservice.InputMethod.AccessibilityInputConnection, keyCode: Int) {
        val t = SystemClock.uptimeMillis()
        c.sendKeyEvent(KeyEvent(t, t, KeyEvent.ACTION_DOWN, keyCode, 0))
        c.sendKeyEvent(KeyEvent(t, t, KeyEvent.ACTION_UP, keyCode, 0))
    }

    /** Keyboard keys through the input connection; false if not handled here. */
    private fun keyViaInputMethod(code: Int): Boolean {
        if (Build.VERSION.SDK_INT < 33) return false
        val c = conn() ?: return false
        if (ctrl) {
            val id = when (code) {
                0x1E -> android.R.id.selectAll // A
                0x2E -> android.R.id.copy // C
                0x2F -> android.R.id.paste // V
                0x2D -> android.R.id.cut // X
                else -> return false
            }
            c.performContextMenuAction(id)
            return true
        }
        val keyCode = when (code) {
            0x0E -> KeyEvent.KEYCODE_DEL // Backspace
            0x1C, 0xE01C -> KeyEvent.KEYCODE_ENTER
            0x0F -> KeyEvent.KEYCODE_TAB
            0xE053 -> KeyEvent.KEYCODE_FORWARD_DEL // Delete
            0xE04B -> KeyEvent.KEYCODE_DPAD_LEFT
            0xE04D -> KeyEvent.KEYCODE_DPAD_RIGHT
            0xE048 -> KeyEvent.KEYCODE_DPAD_UP
            0xE050 -> KeyEvent.KEYCODE_DPAD_DOWN
            0xE047 -> KeyEvent.KEYCODE_MOVE_HOME
            0xE04F -> KeyEvent.KEYCODE_MOVE_END
            else -> return false
        }
        press(c, keyCode)
        return true
    }

    private fun key(code: Int, pressed: Boolean) {
        if (code == 0x1D || code == 0xE01D) {
            ctrl = pressed
            return
        }
        if (!pressed) return
        // Keep order: what was typed before this key goes in first.
        flushTyped()
        if (keyViaInputMethod(code)) return
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
        private var segments = 0
        private var since = 0L

        @Synchronized fun segment() {
            segments++
        }

        @Synchronized fun reset() {
            n = 0; last = 0L; frames = 0; segments = 0; since = SystemClock.uptimeMillis()
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
                    "CONTROL timing moves/s=${n * 1000 / span} gap_ms p50=${g[n / 2]} p95=${g[(n * 95) / 100]} max=${g[n - 1]} frames/s=${frames * 1000 / span} touch_segments/s=${segments * 1000 / span}",
                )
            }
            n = 0; frames = 0; segments = 0; since = now
        }
    }

    companion object {
        /** The running service, so a node started later can register it. */
        @Volatile var instance: ControlService? = null

        /** Turned on in Accessibility settings (it may still not be running:
         *  after its process is killed Android can leave it stopped). */
        fun enabledInSettings(ctx: android.content.Context): Boolean =
            android.provider.Settings.Secure.getString(ctx.contentResolver, android.provider.Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES)
                ?.split(':')?.any { it.startsWith("${ctx.packageName}/") } == true
    }
}
