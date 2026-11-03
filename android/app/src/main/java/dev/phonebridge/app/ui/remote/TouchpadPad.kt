package dev.phonebridge.app.ui.remote

import android.os.SystemClock
import android.util.Log
import androidx.compose.foundation.background
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.systemGestureExclusion
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import dev.phonebridge.app.APP_NAME
import dev.phonebridge.app.Bridge
import dev.phonebridge.app.Radius
import dev.phonebridge.app.ui.components.cardColor
import kotlinx.coroutines.launch
import uniffi.pb_ffi.PbInput
import kotlin.math.abs
import kotlin.math.hypot

private const val TAG = "PhoneBridge"

/** Pointer travel per dp of finger travel, before the speed gain. */
internal const val POINTER_SCALE = 1.6f

/** Faster swipes travel further (light acceleration on top of Windows'). [speed] is dp per event. */
internal fun pointerGain(speed: Float): Float = 1.1f + (speed / 12f).coerceAtMost(1.4f)

/** PC set-1 scan codes for the keys the phone keyboard cannot type. */
private object Scan {
    const val ESC = 0x01
    const val BACKSPACE = 0x0E
    const val TAB = 0x0F
    const val ENTER = 0x1C
    const val UP = 0xE048
    const val LEFT = 0xE04B
    const val RIGHT = 0xE04D
    const val DOWN = 0xE050
    const val WIN = 0xE05B
    const val VOL_DOWN = 0xE02E
    const val VOL_UP = 0xE030
    const val PLAY = 0xE022
}

/**
 * One input session to the paired PC, held while the owning lifecycle is started.
 * [generation] is bumped on every start and stop, so a connect from an earlier start is dropped.
 */
private class TouchpadSession {
    @Volatile var input: PbInput? = null
    private val generation = java.util.concurrent.atomic.AtomicInteger(0)
    var status by mutableStateOf("Connecting…")

    fun start() {
        val gen = generation.incrementAndGet()
        status = "Connecting…"
        if (!Bridge.shownOn()) {
            status = "$APP_NAME is off. Turn it on first"
            return
        }
        Bridge.scope.launch {
            val node = Bridge.awaitNode()
            val peer = node?.peers()?.firstOrNull()
            if (node == null || peer == null) {
                status = if (node == null) "$APP_NAME did not start. Try again" else "Pair with your PC first"
                return@launch
            }
            try {
                val session = node.startInput(peer.id)
                synchronized(generation) {
                    if (generation.get() != gen) {
                        // The user left (or came back) while we were connecting.
                        session.stop()
                        return@launch
                    }
                    input = session
                }
                status = "Controlling ${peer.name}"
                Log.i(TAG, "INPUT started to ${peer.name}")
            } catch (t: Throwable) {
                val declined = t.message?.contains("declined") == true
                status = if (declined) {
                    "${peer.name} did not allow control. Turn on “Let my phone control this PC” in $APP_NAME settings on the PC."
                } else {
                    "Could not reach ${peer.name}. Is $APP_NAME running there?"
                }
                Log.i(TAG, "INPUT refused: $t")
            }
        }
    }

    fun stop() {
        var had = false
        synchronized(generation) {
            generation.incrementAndGet()
            had = input != null
            input?.stop()
            input = null
        }
        if (had) Log.i(TAG, "INPUT stopped")
    }

    fun key(code: Int) {
        input?.let {
            it.key(code.toUShort(), true)
            it.key(code.toUShort(), false)
        }
    }
}

/**
 * The phone as the PC's touchpad and keyboard. One finger moves, tap clicks,
 * two-finger tap right-clicks, two fingers scroll, tap-then-drag drags.
 * The session runs while the lifecycle owner (the activity, or the tab's back stack entry) is started.
 */
@Composable
fun TouchpadPad(modifier: Modifier = Modifier) {
    val session = remember { TouchpadSession() }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val view = LocalView.current
    DisposableEffect(lifecycle, view) {
        val obs = LifecycleEventObserver { _, e ->
            when (e) {
                Lifecycle.Event.ON_START -> {
                    // Held only while the pad is visible (the activity host sets its own window flag too).
                    view.keepScreenOn = true
                    session.start()
                }
                Lifecycle.Event.ON_STOP -> {
                    view.keepScreenOn = false
                    session.stop()
                }
                else -> {}
            }
        }
        lifecycle.addObserver(obs)
        onDispose {
            lifecycle.removeObserver(obs)
            view.keepScreenOn = false
            session.stop()
        }
    }
    var keyboard by remember { mutableStateOf(false) }
    Column(modifier.imePadding()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                session.status,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                modifier = Modifier.weight(1f),
            )
            TextButton(onClick = { keyboard = !keyboard }) { Text(if (keyboard) "Hide keyboard" else "Keyboard") }
        }
        // The gesture area is full width (inside the horizontal safe insets) so a swipe that starts at the screen
        // edge, in the gutter beside the card, still lands on the pad; the card is drawn inset inside it.
        Pad(session, Modifier.weight(1f).fillMaxWidth().windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal)))
        Column(Modifier.padding(horizontal = 16.dp)) {
            Row(Modifier.fillMaxWidth().height(56.dp).padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                MouseButton(session, "Left", 0, Modifier.weight(1f))
                MouseButton(session, "Right", 1, Modifier.weight(1f))
            }
            if (keyboard) KeyboardRow(session)
            Box(Modifier.height(8.dp))
        }
    }
}

/** A Material tonal button (the app's button shape, ripple and pressed state) that holds the mouse button while it is pressed. */
@Composable
private fun MouseButton(session: TouchpadSession, label: String, button: Int, modifier: Modifier) {
    val source = remember { MutableInteractionSource() }
    LaunchedEffect(source) {
        var down = false
        try {
            source.interactions.collect { i ->
                when (i) {
                    is PressInteraction.Press -> if (!down) { down = true; session.input?.button(button.toUByte(), true) }
                    is PressInteraction.Release, is PressInteraction.Cancel -> if (down) { down = false; session.input?.button(button.toUByte(), false) }
                    else -> {}
                }
            }
        } finally {
            // Never leave the PC's button held if the screen goes away mid-press.
            if (down) session.input?.button(button.toUByte(), false)
        }
    }
    FilledTonalButton(onClick = {}, interactionSource = source, modifier = modifier.fillMaxSize()) { Text(label) }
}

@Composable
private fun Pad(session: TouchpadSession, modifier: Modifier) {
    val density = LocalDensity.current.density
    Box(
        modifier
            // Swipes that start at the screen edge move the PC cursor instead of triggering Android's Back gesture.
            // Android honours at most 200 dp of exclusion height per edge and drops the rest unpredictably, so ask
            // for a deterministic band instead of the whole bounds: 200 dp centred vertically, along both edges.
            // This box is full width, so the band reaches the screen edge even though the card is inset.
            .systemGestureExclusion { c ->
                val h = minOf(c.size.height.toFloat(), 200 * density)
                val top = (c.size.height - h) / 2f
                Rect(0f, top, c.size.width.toFloat(), top + h)
            }
            .pointerInput(Unit) {
                var lastTapUp = 0L
                var lastTapPos = Offset.Zero
                var remX = 0f
                var remY = 0f
                var remScroll = 0f
                val slop = 8 * density
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    val t0 = SystemClock.uptimeMillis()
                    // Tap, then touch again quickly: drag with the button held.
                    val dragging = t0 - lastTapUp < 280 && (down.position - lastTapPos).getDistance() < 60 * density
                    if (dragging) session.input?.button(0u, true)
                    var moved = 0f
                    var maxFingers = 1
                    do {
                        val ev = awaitPointerEvent()
                        val pressed = ev.changes.filter { it.pressed }
                        maxFingers = maxOf(maxFingers, pressed.size)
                        if (pressed.size == 1) {
                            val c = pressed[0]
                            val d = c.position - c.previousPosition
                            moved += d.getDistance()
                            val speed = hypot(d.x, d.y) / density
                            val gain = pointerGain(speed)
                            remX += d.x / density * gain * POINTER_SCALE
                            remY += d.y / density * gain * POINTER_SCALE
                            val dx = remX.toInt()
                            val dy = remY.toInt()
                            if (dx != 0 || dy != 0) {
                                session.input?.moveBy(dx.toShort(), dy.toShort())
                                remX -= dx
                                remY -= dy
                            }
                        } else if (pressed.size >= 2) {
                            val dy = pressed.map { (it.position - it.previousPosition).y }.average().toFloat()
                            moved += abs(dy)
                            // Natural scrolling: content follows the fingers.
                            remScroll += dy / density * 4f
                            val w = remScroll.toInt()
                            if (w != 0) {
                                session.input?.scroll(0, w.toShort())
                                remScroll -= w
                            }
                        }
                        ev.changes.forEach { it.consume() }
                    } while (ev.changes.any { it.pressed })
                    val now = SystemClock.uptimeMillis()
                    if (dragging) {
                        session.input?.button(0u, false)
                    } else if (moved < slop && now - t0 < 250) {
                        val b: UByte = if (maxFingers >= 2) 1u else 0u
                        session.input?.button(b, true)
                        session.input?.button(b, false)
                        if (maxFingers == 1) {
                            lastTapUp = now
                            lastTapPos = down.position
                        }
                    }
                }
            }
            .padding(horizontal = 16.dp, vertical = 4.dp),
    ) {
        // The card: same gutter, radius and tonal surface as the cards on the other tabs.
        Box(
            Modifier.fillMaxSize().clip(RoundedCornerShape(Radius.card)).background(cardColor()),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                "Move with one finger · tap to click\nTwo fingers to scroll · two-finger tap to right-click",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(24.dp),
            )
        }
    }
}

/** Hidden text field drives the soft keyboard; changes become key events. */
@Composable
private fun KeyboardRow(session: TouchpadSession) {
    val sentinel = " "
    var value by remember { mutableStateOf(TextFieldValue(sentinel, TextRange(1))) }
    val focus = remember { FocusRequester() }
    val kb = LocalSoftwareKeyboardController.current
    LaunchedEffect(Unit) {
        focus.requestFocus()
        kb?.show()
    }
    BasicTextField(
        value = value,
        onValueChange = { v ->
            val t = v.text
            when {
                t.length < sentinel.length -> session.key(Scan.BACKSPACE)
                t.startsWith(sentinel) && t.length > sentinel.length -> session.input?.typeText(t.substring(sentinel.length))
            }
            value = TextFieldValue(sentinel, TextRange(1))
        },
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Text, imeAction = ImeAction.Send, autoCorrectEnabled = false),
        keyboardActions = KeyboardActions(onSend = { session.key(Scan.ENTER) }),
        modifier = Modifier.size(1.dp).focusRequester(focus),
    )
    val keys = listOf(
        "Esc" to Scan.ESC, "Tab" to Scan.TAB, "←" to Scan.LEFT, "↑" to Scan.UP,
        "↓" to Scan.DOWN, "→" to Scan.RIGHT, "Enter" to Scan.ENTER, "⌫" to Scan.BACKSPACE,
        "Win" to Scan.WIN, "Vol −" to Scan.VOL_DOWN, "Vol +" to Scan.VOL_UP, "⏯" to Scan.PLAY,
    )
    LazyRow(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        items(keys) { (label, code) ->
            FilledTonalButton(onClick = { session.key(code) }) { Text(label) }
        }
    }
}
