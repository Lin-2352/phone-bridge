package dev.phonebridge.app

import android.content.Context
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.projection.MediaProjection
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.util.Log
import android.view.Display
import android.view.Surface
import android.view.WindowManager
import androidx.compose.runtime.mutableStateOf
import java.nio.ByteBuffer
import java.nio.ByteOrder
import uniffi.pb_ffi.PbScreen
import uniffi.pb_ffi.PbScreenAccept
import uniffi.pb_ffi.ScreenListener
import uniffi.pb_ffi.chooseCodec
import uniffi.pb_ffi.fitSize
import uniffi.pb_ffi.nextCodec

private const val TAG = "PhoneBridge"
private const val FLAG_KEY: UByte = 1u
private const val FLAG_CONFIG: UByte = 2u
/** After a dropped link the capture stays (paused) this long for a resume. */
private const val RESUME_GRACE_MS = 30_000L
/** Step-down size (720-class) and frame rate. */
private const val STEP_DOWN_LONG = 1280
private const val STEP_DOWN_FPS = 15
/** Frames per second encoded (the 120 Hz panel is capped to this). */
private const val FPS = 60
/** QP ceiling: fast motion degrades in size, never in blocks. */
private const val QP_MAX = 38

/** What the PC asked for. */
data class ScreenAsk(
    val id: ULong,
    val peer: String,
    val maxLong: Int,
    val sharp: Boolean,
    val dim: Boolean,
    val codecs: List<Int>,
)

/**
 * The phone's screen into the hardware encoder: a virtual display renders
 * straight into the encoder's input surface (no copies); each encoded unit
 * goes to [PbScreen], which paces it to the PC.
 */
class ScreenCapture(
    private val ctx: Context,
    private val projection: MediaProjection,
    private val ask: ScreenAsk,
) {
    private val caps = EncoderCaps.get()
    private val thread = HandlerThread("pb-screen").apply { start() }
    private val handler = Handler(thread.looper)
    private var codec: Int = 0
    private var maxLong: Int = if (ask.sharp || ask.maxLong == 0) 0 else ask.maxLong
    private var fps = FPS
    private var width = 0
    private var height = 0
    private var rotation = 0
    private var encoder: MediaCodec? = null
    private var inputSurface: Surface? = null
    private var display: VirtualDisplay? = null
    @Volatile private var screen: PbScreen? = null
    @Volatile private var paused = false
    private var steppedDown = false
    private var startedAt = 0L
    private var firstOutput = true
    /** Per-second counters for the timing log. */
    private var statFrames = 0
    private var statBytes = 0L
    private var statDrops = 0
    private var statKeys = 0
    private var statSince = 0L
    private var bitrate = 0

    private val displays = ctx.getSystemService(DisplayManager::class.java)
    private val displayListener = object : DisplayManager.DisplayListener {
        override fun onDisplayAdded(displayId: Int) = Unit
        override fun onDisplayRemoved(displayId: Int) = Unit
        override fun onDisplayChanged(displayId: Int) {
            if (displayId == Display.DEFAULT_DISPLAY && currentRotation() != rotation) handler.post { reconfigure() }
        }
    }

    /** Choose codec and size; returns the answer for the PC. Main thread. */
    fun prepare(): PbScreenAccept? {
        val chosen = chooseCodec(bytes(ask.codecs), bytes(caps.codecs)) ?: return null
        codec = chosen.toInt()
        rotation = currentRotation()
        measure()
        projection.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() {
                Log.i(TAG, "SCREEN projection stopped by the system")
                ScreenShare.stop(userStopped = true)
            }
        }, Handler(Looper.getMainLooper()))
        startEncoder()
        display = projection.createVirtualDisplay(
            "Phone Bridge", width, height, density(),
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, inputSurface, null, handler,
        )
        displays.registerDisplayListener(displayListener, handler)
        return accept()
    }

    private fun bytes(codecs: List<Int>) = ByteArray(codecs.size) { codecs[it].toByte() }

    fun accept() = PbScreenAccept(codec.toUByte(), width.toUInt(), height.toUInt(), rotation.toUShort())

    /** Frames from now on go to this session (a new one after a resume). */
    fun attach(s: PbScreen) {
        screen = s
        resume()
    }

    fun pause() {
        paused = true
        handler.post { display?.surface = null }
    }

    fun resume() {
        paused = false
        handler.post {
            display?.surface = inputSurface
            requestKey()
        }
    }

    /** Memory or heat: 720-class at 15 fps for the rest of the session. */
    fun stepDown() = handler.post {
        if (steppedDown) return@post
        steppedDown = true
        maxLong = STEP_DOWN_LONG
        fps = STEP_DOWN_FPS
        reconfigure()
        screen?.notice("Lower quality to save memory")
        Log.i(TAG, "SCREEN stepped down to ${width}x$height at $fps fps")
    }

    /** The PC could not decode: its next codec, if any. */
    fun nextCodecOrStop() = handler.post {
        val next = nextCodec(codec.toUByte(), bytes(ask.codecs))?.toInt()
        if (next == null || next !in caps.codecs) {
            Log.i(TAG, "SCREEN no codec left for the PC")
            ScreenShare.stop(userStopped = true)
            return@post
        }
        codec = next
        reconfigure()
    }

    fun stop() {
        runCatching { displays.unregisterDisplayListener(displayListener) }
        handler.post {
            runCatching { display?.release() }
            stopEncoder()
            runCatching { projection.stop() }
            thread.quitSafely()
        }
    }

    private fun currentRotation(): Int = (displays.getDisplay(Display.DEFAULT_DISPLAY)?.rotation ?: 0) * 90

    private fun density(): Int = ctx.resources.displayMetrics.densityDpi

    private fun measure() {
        val (sw, sh) = if (Build.VERSION.SDK_INT >= 30) {
            val b = ctx.getSystemService(WindowManager::class.java).currentWindowMetrics.bounds
            b.width() to b.height()
        } else {
            val p = android.graphics.Point()
            @Suppress("DEPRECATION")
            displays.getDisplay(Display.DEFAULT_DISPLAY)?.getRealSize(p)
            p.x to p.y
        }
        val size = fitSize(sw.toUInt(), sh.toUInt(), maxLong.toUInt(), caps.alignW.toUInt(), caps.alignH.toUInt())
        width = size[0].toInt()
        height = size[1].toInt()
    }

    private fun format(): MediaFormat {
        val mime = caps.mime(codec)
        val bitrate = if (codec == CODEC_H264) 16_000_000 else 12_000_000
        return MediaFormat.createVideoFormat(mime, width, height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
            setInteger(
                MediaFormat.KEY_BITRATE_MODE,
                if (caps.vbr) MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_VBR else MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR,
            )
            setInteger(MediaFormat.KEY_FRAME_RATE, fps)
            // The virtual display follows the 120 Hz panel; without this the
            // encoder gets every display frame and queues them (lag).
            setFloat("max-fps-to-encoder", fps.toFloat())
            // No periodic key frames (no pulsing): only when asked.
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 3600)
            // A few sharpening repeats once the screen stops changing.
            setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
            setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
            setInteger(MediaFormat.KEY_PRIORITY, 0)
            if (Build.VERSION.SDK_INT >= 30) setInteger(MediaFormat.KEY_LATENCY, 1)
            setInteger(MediaFormat.KEY_COLOR_STANDARD, MediaFormat.COLOR_STANDARD_BT709)
            setInteger(MediaFormat.KEY_COLOR_TRANSFER, MediaFormat.COLOR_TRANSFER_SDR_VIDEO)
            setInteger(MediaFormat.KEY_COLOR_RANGE, MediaFormat.COLOR_RANGE_LIMITED)
            if (codec == CODEC_H265_MAIN10) {
                setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.HEVCProfileMain10)
            }
            if (caps.qpMax && Build.VERSION.SDK_INT >= 31) setInteger(MediaFormat.KEY_VIDEO_QP_MAX, QP_MAX)
        }
    }

    private fun startEncoder() {
        val enc = MediaCodec.createEncoderByType(caps.mime(codec))
        enc.setCallback(object : MediaCodec.Callback() {
            override fun onInputBufferAvailable(codec: MediaCodec, index: Int) = Unit
            override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
                onOutput(codec, index, info)
            }
            override fun onError(codec: MediaCodec, e: MediaCodec.CodecException) {
                Log.e(TAG, "SCREEN encoder error: ${e.diagnosticInfo}")
            }
            override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) = Unit
        }, handler)
        enc.configure(format(), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        inputSurface = enc.createInputSurface()
        enc.start()
        encoder = enc
        startedAt = android.os.SystemClock.uptimeMillis()
        firstOutput = true
        bitrate = if (codec == CODEC_H264) 16_000_000 else 12_000_000
        Log.i(TAG, "SCREEN encoder ${caps.encoderName} codec=$codec ${width}x$height rot=$rotation fps=$fps vbr=${caps.vbr}")
    }

    private fun stopEncoder() {
        runCatching { encoder?.stop() }
        runCatching { encoder?.release() }
        runCatching { inputSurface?.release() }
        encoder = null
        inputSurface = null
    }

    /** New size, rotation or codec: a fresh encoder; its config record follows. */
    private fun reconfigure() {
        rotation = currentRotation()
        measure()
        display?.surface = null
        stopEncoder()
        startEncoder()
        display?.resize(width, height, density())
        if (!paused) display?.surface = inputSurface
    }

    private fun requestKey() {
        runCatching { encoder?.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) }) }
    }

    private fun onOutput(enc: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
        try {
            val buf = enc.getOutputBuffer(index) ?: return
            if (info.size <= 0) return
            val data = ByteArray(info.size)
            buf.position(info.offset)
            buf.get(data, 0, info.size)
            val s = screen ?: return
            if (paused) return
            val config = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
            val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
            val now = android.os.SystemClock.uptimeMillis()
            val flags: UByte = when {
                config -> FLAG_CONFIG
                key -> FLAG_KEY
                else -> 0u
            }
            val payload = if (config) configPayload(data) else data
            if (firstOutput) {
                firstOutput = false
                Log.i(TAG, "SCREEN first output after ${now - startedAt} ms")
            }
            val advice = s.sendFrame(payload, info.presentationTimeUs, flags)
            stats(now, info.size, advice.sent, advice.requestKey)
            if (advice.requestKey) requestKey()
            if (advice.newBitrate != 0u) {
                bitrate = advice.newBitrate.toInt()
                runCatching {
                    enc.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_VIDEO_BITRATE, advice.newBitrate.toInt()) })
                }
            }
        } finally {
            runCatching { enc.releaseOutputBuffer(index, false) }
        }
    }

    /** One line a second: frames out, bitrate actually produced, pacer drops. */
    private fun stats(now: Long, bytes: Int, sent: Boolean, keyAsked: Boolean) {
        if (statSince == 0L) statSince = now
        statFrames++
        statBytes += bytes
        if (!sent) statDrops++
        if (keyAsked) statKeys++
        val span = now - statSince
        if (span >= 5000) {
            Log.i(
                TAG,
                "SCREEN stats out=${statFrames * 1000 / span}/s ${statBytes * 8 / span} kbit/s drops=$statDrops keys=$statKeys target=${bitrate / 1000} kbit/s",
            )
            statFrames = 0
            statBytes = 0
            statDrops = 0
            statKeys = 0
            statSince = now
        }
    }

    /** `u16 rotation | u16 width | u16 height | headers`, little-endian. */
    private fun configPayload(headers: ByteArray): ByteArray =
        ByteBuffer.allocate(6 + headers.size).order(ByteOrder.LITTLE_ENDIAN).apply {
            putShort(rotation.toShort())
            putShort(width.toShort())
            putShort(height.toShort())
            put(headers)
        }.array()
}

/** The running screen share (one at a time) and its PC-side events. */
object ScreenShare : ScreenListener {
    /** Name of the PC seeing this screen, or null. */
    val sharingWith = mutableStateOf<String?>(null)

    @Volatile var capture: ScreenCapture? = null
        private set
    @Volatile private var screen: PbScreen? = null
    @Volatile private var ask: ScreenAsk? = null
    private val main = Handler(Looper.getMainLooper())
    private val graceEnd = Runnable { stop(userStopped = false) }

    /** Consent and the capture prompt are done: start capturing. */
    fun start(ctx: Context, projection: MediaProjection, a: ScreenAsk): Boolean {
        val node = Bridge.node ?: return false
        val c = ScreenCapture(ctx.applicationContext, projection, a)
        val accept = runCatching { c.prepare() }.onFailure { Log.e(TAG, "SCREEN capture failed: $it") }.getOrNull()
        if (accept == null) {
            node.answerScreen(a.id, null)
            c.stop()
            return false
        }
        val s = node.answerScreen(a.id, accept)
        if (s == null) {
            c.stop()
            return false
        }
        node.setScreenListener(this)
        capture = c
        screen = s
        ask = a
        lastActive = true
        lastDim = null
        c.attach(s)
        sharingWith.value = a.peer
        notifySharing(ctx, a.peer)
        Log.i(TAG, "SCREEN sharing with ${a.peer}")
        return true
    }

    /** The same PC asked again within the grace: carry on, no prompt. */
    fun resume(id: ULong): Boolean {
        val c = capture ?: return false
        val node = Bridge.node ?: return false
        main.removeCallbacks(graceEnd)
        val s = node.answerScreen(id, c.accept()) ?: return false
        screen = s
        c.attach(s)
        sharingWith.value = ask?.peer
        Log.i(TAG, "SCREEN resumed")
        return true
    }

    /** End sharing; the projection service type is dropped. */
    fun stop(userStopped: Boolean) {
        main.removeCallbacks(graceEnd)
        if (userStopped) runCatching { screen?.stop() }
        capture?.stop()
        capture = null
        screen = null
        sharingWith.value = null
        PbService.instance?.let { notifySharing(it, null) }
        PbService.instance?.leaveProjection()
        ControlService.instance?.endDim()
    }

    @Volatile private var lastActive = true
    @Volatile private var lastDim: Boolean? = null

    override fun onState(active: Boolean, dim: Boolean) {
        val c = capture ?: return
        // Only changes: a resume asks the encoder for a key frame.
        if (active != lastActive) {
            lastActive = active
            if (active) c.resume() else c.pause()
            Log.i(TAG, "SCREEN window ${if (active) "visible" else "minimised"}")
        }
        if (dim != lastDim) {
            lastDim = dim
            main.post { ControlService.instance?.setDim(dim) }
        }
    }

    override fun onCodecFailed() {
        capture?.nextCodecOrStop()
    }

    override fun onTouch(phase: UByte, x: UShort, y: UShort) {
        main.post { ControlService.instance?.touchAt(phase.toInt(), x.toInt(), y.toInt()) }
    }

    override fun onPinch(out: Boolean, x: UShort, y: UShort) {
        main.post { ControlService.instance?.pinchAt(out, x.toInt(), y.toInt()) }
    }

    override fun onKey(scancode: UShort, pressed: Boolean) {
        ControlService.instance?.onInput(3u, scancode.toInt(), 0, pressed)
    }

    override fun onChar(unit: UShort) {
        ControlService.instance?.onInput(4u, unit.toInt(), 0, false)
    }

    override fun onButton(button: UByte) {
        ControlService.instance?.onInput(1u, button.toInt(), 0, true)
    }

    override fun onWheel(dx: Short, dy: Short) {
        ControlService.instance?.onInput(2u, dx.toInt(), dy.toInt(), false)
    }

    override fun onStopped(resumable: Boolean) {
        main.post {
            if (resumable && capture != null) {
                // The link dropped: keep capturing (paused) for a resume.
                capture?.pause()
                main.postDelayed(graceEnd, RESUME_GRACE_MS)
                Log.i(TAG, "SCREEN link lost; waiting for a resume")
            } else {
                stop(userStopped = false)
            }
        }
    }
}
