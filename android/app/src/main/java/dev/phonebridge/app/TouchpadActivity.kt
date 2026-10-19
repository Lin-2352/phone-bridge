package dev.phonebridge.app

import android.os.Bundle
import android.os.SystemClock
import android.util.Log
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
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
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import uniffi.pb_ffi.PbInput
import kotlin.math.abs
import kotlin.math.hypot

private const val TAG = "PhoneBridge"

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
 * The phone as the PC's touchpad and keyboard. One finger moves, tap clicks,
 * two-finger tap right-clicks, two fingers scroll, tap-then-drag drags.
 * The session runs while this screen is visible.
 */
class TouchpadActivity : ComponentActivity() {
    @Volatile private var input: PbInput? = null
    @Volatile private var visible = false
    /** Bumped on every start and stop, so a connect from an earlier start is dropped. */
    private val generation = java.util.concurrent.atomic.AtomicInteger(0)
    private val status = mutableStateOf("Connecting…")

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        ThemeChoice.mode.intValue = Prefs(this).theme
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        setContent { PbTheme { Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) { Screen() } } }
    }

    override fun onStart() {
        super.onStart()
        visible = true
        val gen = generation.incrementAndGet()
        status.value = "Connecting\u2026"
        if (!Bridge.shownOn()) {
            status.value = "Phone Bridge is off. Turn it on first"
            return
        }
        Bridge.scope.launch {
            val node = Bridge.awaitNode()
            val peer = node?.peers()?.firstOrNull()
            if (node == null || peer == null) {
                status.value = if (node == null) "Phone Bridge did not start. Try again" else "Pair with your PC first"
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
                status.value = "Controlling ${peer.name}"
                Log.i(TAG, "INPUT started to ${peer.name}")
            } catch (t: Throwable) {
                val declined = t.message?.contains("declined") == true
                status.value = if (declined) {
                    "${peer.name} did not allow control. Turn on \u201cLet my phone control this PC\u201d in Phone Bridge settings on the PC."
                } else {
                    "Could not reach ${peer.name}. Is Phone Bridge running there?"
                }
                Log.i(TAG, "INPUT refused: $t")
            }
        }
    }

    override fun onStop() {
        visible = false
        synchronized(generation) {
            generation.incrementAndGet()
            input?.stop()
            input = null
        }
        Log.i(TAG, "INPUT stopped")
        super.onStop()
    }

    private fun key(code: Int) {
        input?.let {
            it.key(code.toUShort(), true)
            it.key(code.toUShort(), false)
        }
    }

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    private fun Screen() {
        var keyboard by remember { mutableStateOf(false) }
        Scaffold(
            topBar = {
                TopAppBar(
                    title = {
                        Column {
                            Text("Touchpad")
                            Text(status.value, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2)
                        }
                    },
                    navigationIcon = { IconButton(onClick = { finish() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } },
                    actions = { TextButton(onClick = { keyboard = !keyboard }) { Text(if (keyboard) "Hide keyboard" else "Keyboard") } },
                )
            },
        ) { pad ->
            Column(Modifier.padding(pad).fillMaxSize().navigationBarsPadding().imePadding().padding(horizontal = 16.dp)) {
                Pad(Modifier.weight(1f).fillMaxWidth())
                Row(Modifier.fillMaxWidth().height(64.dp).padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    MouseButton("Left", 0, Modifier.weight(1f))
                    MouseButton("Right", 1, Modifier.weight(1f))
                }
                if (keyboard) KeyboardRow()
                Box(Modifier.height(8.dp))
            }
        }
    }

    @Composable
    private fun MouseButton(label: String, button: Int, modifier: Modifier) {
        Box(
            modifier
                .fillMaxSize()
                .clip(RoundedCornerShape(16.dp))
                .background(MaterialTheme.colorScheme.secondaryContainer)
                .pointerInput(button) {
                    awaitEachGesture {
                        awaitFirstDown()
                        input?.button(button.toUByte(), true)
                        do {
                            val e = awaitPointerEvent()
                        } while (e.changes.any { it.pressed })
                        input?.button(button.toUByte(), false)
                    }
                },
            contentAlignment = Alignment.Center,
        ) { Text(label, color = MaterialTheme.colorScheme.onSecondaryContainer) }
    }

    @Composable
    private fun Pad(modifier: Modifier) {
        val density = resources.displayMetrics.density
        Box(
            modifier
                .clip(RoundedCornerShape(24.dp))
                .background(MaterialTheme.colorScheme.surfaceContainer)
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
                        if (dragging) input?.button(0u, true)
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
                                // Faster swipes travel further (light acceleration on top of Windows').
                                val speed = hypot(d.x, d.y) / density
                                val gain = 1.1f + (speed / 12f).coerceAtMost(1.4f)
                                remX += d.x / density * gain * 1.6f
                                remY += d.y / density * gain * 1.6f
                                val dx = remX.toInt()
                                val dy = remY.toInt()
                                if (dx != 0 || dy != 0) {
                                    input?.moveBy(dx.toShort(), dy.toShort())
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
                                    input?.scroll(0, w.toShort())
                                    remScroll -= w
                                }
                            }
                            ev.changes.forEach { it.consume() }
                        } while (ev.changes.any { it.pressed })
                        val now = SystemClock.uptimeMillis()
                        if (dragging) {
                            input?.button(0u, false)
                        } else if (moved < slop && now - t0 < 250) {
                            val b: UByte = if (maxFingers >= 2) 1u else 0u
                            input?.button(b, true)
                            input?.button(b, false)
                            if (maxFingers == 1) {
                                lastTapUp = now
                                lastTapPos = down.position
                            }
                        }
                    }
                },
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

    /** Hidden text field drives the soft keyboard; changes become key events. */
    @Composable
    private fun KeyboardRow() {
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
                    t.length < sentinel.length -> key(Scan.BACKSPACE)
                    t.startsWith(sentinel) && t.length > sentinel.length -> input?.typeText(t.substring(sentinel.length))
                }
                value = TextFieldValue(sentinel, TextRange(1))
            },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Text, imeAction = ImeAction.Send, autoCorrectEnabled = false),
            keyboardActions = KeyboardActions(onSend = { key(Scan.ENTER) }),
            modifier = Modifier.size(1.dp).focusRequester(focus),
        )
        val keys = listOf(
            "Esc" to Scan.ESC, "Tab" to Scan.TAB, "←" to Scan.LEFT, "↑" to Scan.UP,
            "↓" to Scan.DOWN, "→" to Scan.RIGHT, "Enter" to Scan.ENTER, "⌫" to Scan.BACKSPACE,
            "Win" to Scan.WIN, "Vol −" to Scan.VOL_DOWN, "Vol +" to Scan.VOL_UP, "⏯" to Scan.PLAY,
        )
        LazyRow(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            items(keys) { (label, code) ->
                FilledTonalButton(onClick = { key(code) }) { Text(label) }
            }
        }
    }
}
