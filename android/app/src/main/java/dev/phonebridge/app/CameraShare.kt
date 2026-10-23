package dev.phonebridge.app

import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.display.DisplayManager
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.util.Log
import android.util.Range
import android.view.Display
import android.view.OrientationEventListener
import android.view.Surface
import androidx.compose.runtime.mutableStateOf
import java.nio.ByteBuffer
import java.nio.ByteOrder
import uniffi.pb_ffi.CameraListener
import uniffi.pb_ffi.PbCameraAccept
import uniffi.pb_ffi.PbScreen
import uniffi.pb_ffi.chooseCodec

private const val TAG = "PhoneBridge"
private const val FLAG_KEY: UByte = 1u
private const val FLAG_CONFIG: UByte = 2u
private const val CAMERA_FPS = 30
/** The camera is kept this long after a dropped link, for a resume. */
private const val CAMERA_GRACE_MS = 30_000L

/** Whether the last session started stepped down: the hysteresis carries over between sessions. */
@Volatile private var cameraSteppedDown = false

/** What the PC asked for: the lens and the 16:9 size, with the codecs it decodes. */
data class CameraAsk(
    val id: ULong,
    val peer: String,
    /** 0 back, 1 front. */
    val lens: Int,
    val width: Int,
    val height: Int,
    val codecs: List<Int>,
)

/**
 * The phone's camera for one session: Camera2 renders straight into the
 * hardware encoder's input surface; encoded units go to the PC through the
 * same `PbScreen` sender the screen uses (pacer, byte queue, resume).
 */
class CameraCapture(private val ctx: Context, private var ask: CameraAsk) {
    private val caps = EncoderCaps.get()
    private val thread = HandlerThread("pb-camera").apply { start() }
    private val handler = Handler(thread.looper)
    private val manager = ctx.getSystemService(CameraManager::class.java)
    private val displays = ctx.getSystemService(DisplayManager::class.java)
    private var codec = 0
    var lens = ask.lens
        private set
    private var width = 1920
    private var height = 1080
    private var rotation = 0
    private var encoder: MediaCodec? = null
    private var inputSurface: Surface? = null
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    @Volatile private var sender: PbScreen? = null
    @Volatile private var closed = false
    private var lastHeaders: ByteArray? = null
    private var bitrate = 0

    private val displayListener = object : DisplayManager.DisplayListener {
        override fun onDisplayAdded(displayId: Int) = Unit
        override fun onDisplayRemoved(displayId: Int) = Unit
        override fun onDisplayChanged(displayId: Int) {
            if (displayId != Display.DEFAULT_DISPLAY) return
            handler.post {
                val r = computeRotation()
                if (r != rotation) {
                    rotation = r
                    sendConfig()
                }
            }
        }
    }

    private fun facing(front: Boolean) =
        if (front) CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK

    private fun cameraId(front: Boolean): String? = manager.cameraIdList.firstOrNull {
        manager.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == facing(front)
    }

    /**
     * How the phone is held, from its sensor: while it is a webcam the screen
     * is often off or on a portrait-only screen, so the display rotation
     * would say "portrait" however the phone stands. The display is the
     * fallback when the sensor cannot tell (flat on a table).
     */
    @Volatile private var sensorRotation: Int? = null
    private val orientation = object : OrientationEventListener(ctx) {
        override fun onOrientationChanged(degrees: Int) {
            val r = deviceRotationFromSensor(degrees) ?: return
            if (r == sensorRotation) return
            sensorRotation = r
            handler.post {
                val next = computeRotation()
                if (next != rotation) {
                    rotation = next
                    sendConfig()
                }
            }
        }
    }

    private fun deviceRotation(): Int =
        sensorRotation ?: ((displays.getDisplay(Display.DEFAULT_DISPLAY)?.rotation ?: 0) * 90)

    private fun computeRotation(): Int {
        val id = cameraId(lens == 1) ?: return 0
        val sensor = manager.getCameraCharacteristics(id).get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 0
        return rotationDegrees(sensor, deviceRotation(), front = lens == 1)
    }

    /** Free memory, as Android reports it, in MB. */
    private fun freeMb(): Long {
        val mi = android.app.ActivityManager.MemoryInfo()
        ctx.getSystemService(android.app.ActivityManager::class.java).getMemoryInfo(mi)
        return mi.availMem / 1_048_576
    }

    private fun sizeFor(id: String): Pair<Int, Int>? {
        val map = manager.getCameraCharacteristics(id)
            .get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return null
        val sizes = (map.getOutputSizes(MediaCodec::class.java) ?: map.getOutputSizes(ImageFormat.YUV_420_888) ?: return null)
            .map { it.width to it.height }
        // Decided once, when the session starts: no change mid-stream.
        val down = cameraStepDown(freeMb(), cameraSteppedDown)
        cameraSteppedDown = down
        if (down) Log.i(TAG, "CAMERA low memory: capped at 1280x720")
        return pickSize(sizes, down)
    }

    /** Choose lens, size and codec; start the encoder. Main thread. */
    fun prepare(): PbCameraAccept? {
        val chosen = chooseCodec(bytes(ask.codecs), bytes(caps.codecs)) ?: return null
        codec = chosen.toInt()
        val id = cameraId(lens == 1) ?: cameraId(lens != 1) ?: return null
        val size = sizeFor(id) ?: return null
        width = size.first
        height = size.second
        rotation = computeRotation()
        startEncoder()
        displays.registerDisplayListener(displayListener, handler)
        if (orientation.canDetectOrientation()) orientation.enable()
        return accept()
    }

    private fun bytes(codecs: List<Int>) = ByteArray(codecs.size) { codecs[it].toByte() }

    fun accept() = PbCameraAccept(
        codec.toUByte(), width.toUInt(), height.toUInt(), rotation.toUShort(), lens.toUByte(),
    )

    /** The session is up: send frames to the PC and open the camera. */
    @SuppressLint("MissingPermission")
    fun attach(s: PbScreen) {
        sender = s
        handler.post {
            sendConfig()
            requestKey()
            if (device == null) openCamera()
        }
    }

    /** The link dropped: the camera keeps running, frames are not sent. */
    fun detach() {
        sender = null
    }

    private fun format(): MediaFormat {
        val mime = caps.mime(codec)
        val rate = if (codec == CODEC_H264) 10_000_000 else 8_000_000
        return MediaFormat.createVideoFormat(mime, width, height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, rate)
            setInteger(
                MediaFormat.KEY_BITRATE_MODE,
                if (caps.vbr) MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_VBR else MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR,
            )
            setInteger(MediaFormat.KEY_FRAME_RATE, CAMERA_FPS)
            // No repeat-previous-frame: a camera never stands still. Key
            // frames come on request, and every ten seconds as a safety net.
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 10)
            setInteger(MediaFormat.KEY_COLOR_STANDARD, MediaFormat.COLOR_STANDARD_BT709)
            setInteger(MediaFormat.KEY_COLOR_TRANSFER, MediaFormat.COLOR_TRANSFER_SDR_VIDEO)
            setInteger(MediaFormat.KEY_COLOR_RANGE, MediaFormat.COLOR_RANGE_LIMITED)
            if (codec == CODEC_H265_MAIN10) {
                setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.HEVCProfileMain10)
            }
        }
    }

    private fun startEncoder() {
        lastHeaders = null
        val enc = MediaCodec.createEncoderByType(caps.mime(codec))
        enc.setCallback(object : MediaCodec.Callback() {
            override fun onInputBufferAvailable(codec: MediaCodec, index: Int) = Unit
            override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
                onOutput(codec, index, info)
            }
            override fun onError(codec: MediaCodec, e: MediaCodec.CodecException) {
                Log.e(TAG, "CAMERA encoder error: ${e.diagnosticInfo}")
            }
            override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) = Unit
        }, handler)
        enc.configure(format(), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        inputSurface = enc.createInputSurface()
        enc.start()
        encoder = enc
        bitrate = if (codec == CODEC_H264) 10_000_000 else 8_000_000
        Log.i(TAG, "CAMERA encoder ${caps.encoderName} codec=$codec ${width}x$height rot=$rotation lens=$lens")
    }

    private fun stopEncoder() {
        runCatching { encoder?.stop() }
        runCatching { encoder?.release() }
        runCatching { inputSurface?.release() }
        encoder = null
        inputSurface = null
    }

    /**
     * The camera could not be opened or configured. The phone's own Camera
     * app may still be letting go of the sensor, so try again a few times
     * (0.4 / 0.8 / 1.2 s) before giving up.
     */
    private fun retryOrFail(attempt: Int, why: String) {
        closeCamera()
        val delay = retryDelayMs(attempt + 1)
        if (closed) return
        if (delay == null) {
            Log.e(TAG, "CAMERA giving up: $why")
            android.os.Handler(Looper.getMainLooper()).post {
                android.widget.Toast.makeText(
                    ctx,
                    "The phone's camera is busy. Close the Camera app and try again",
                    android.widget.Toast.LENGTH_LONG,
                ).show()
            }
            CameraShare.cameraFailed()
            return
        }
        Log.i(TAG, "CAMERA retry ${attempt + 1} in $delay ms ($why)")
        handler.postDelayed({ openCamera(attempt + 1) }, delay)
    }

    @SuppressLint("MissingPermission")
    private fun openCamera(attempt: Int = 0) {
        if (closed) return
        if (ctx.checkSelfPermission(android.Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            Log.e(TAG, "CAMERA no permission")
            CameraShare.cameraFailed()
            return
        }
        val id = cameraId(lens == 1) ?: return
        runCatching {
            manager.openCamera(id, object : CameraDevice.StateCallback() {
                override fun onOpened(camera: CameraDevice) {
                    if (closed) return camera.close()
                    device = camera
                    startSession(camera, id, attempt)
                }
                override fun onDisconnected(camera: CameraDevice) {
                    Log.i(TAG, "CAMERA disconnected (another app took it?)")
                    camera.close()
                    if (device === camera) device = null
                    retryOrFail(attempt, "disconnected")
                }
                override fun onError(camera: CameraDevice, error: Int) {
                    camera.close()
                    if (device === camera) device = null
                    retryOrFail(attempt, "error $error")
                }
            }, handler)
        }.onFailure {
            retryOrFail(attempt, "open failed: $it")
        }
    }

    @Suppress("DEPRECATION")
    private fun startSession(camera: CameraDevice, id: String, attempt: Int) {
        val surface = inputSurface ?: return
        runCatching {
            camera.createCaptureSession(
                listOf(surface),
                object : CameraCaptureSession.StateCallback() {
                    override fun onConfigured(s: CameraCaptureSession) {
                        if (closed) return s.close()
                        session = s
                        runCatching { s.setRepeatingRequest(request(camera, id, surface), null, handler) }
                            .onFailure { Log.e(TAG, "CAMERA repeating request failed: $it") }
                    }
                    override fun onConfigureFailed(s: CameraCaptureSession) {
                        retryOrFail(attempt, "session configuration failed")
                    }
                },
                handler,
            )
        }.onFailure { retryOrFail(attempt, "session failed: $it") }
    }

    /** TEMPLATE_RECORD, fps locked to 30, stabilisation on when offered. */
    private fun request(camera: CameraDevice, id: String, surface: Surface): CaptureRequest {
        val ch = manager.getCameraCharacteristics(id)
        val b = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD)
        b.addTarget(surface)
        val ranges = ch.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
            ?.map { it.lower to it.upper }.orEmpty()
        pickFpsRange(ranges)?.let { b.set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(it.first, it.second)) }
        val video = ch.get(CameraCharacteristics.CONTROL_AVAILABLE_VIDEO_STABILIZATION_MODES)?.toList().orEmpty()
        val optical = ch.get(CameraCharacteristics.LENS_INFO_AVAILABLE_OPTICAL_STABILIZATION)?.toList().orEmpty()
        if (CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_ON in video) {
            b.set(CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE, CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_ON)
        } else if (CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE_ON in optical) {
            b.set(CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE, CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE_ON)
        }
        return b.build()
    }

    private fun closeCamera() {
        runCatching { session?.close() }
        runCatching { device?.close() }
        session = null
        device = null
    }

    /** The other lens: close this camera, open that one on the same encoder. */
    fun switchTo(newLens: Int) {
        handler.post {
            if (closed || newLens == lens) return@post
            closeCamera()
            lens = newLens
            val id = cameraId(lens == 1)
            val size = id?.let { sizeFor(it) }
            val newRotation = computeRotation()
            if (size != null && size != (width to height)) {
                // The other camera cannot do this size: a fresh encoder.
                stopEncoder()
                width = size.first
                height = size.second
                rotation = newRotation
                startEncoder()
            } else if (newRotation != rotation) {
                rotation = newRotation
                sendConfig()
            }
            openCamera()
            requestKey()
        }
    }

    /** The headers again with the current rotation and size (a rotation or lens change). */
    private fun sendConfig() {
        val headers = lastHeaders ?: return
        sender?.sendFrame(configPayload(headers), System.nanoTime() / 1000, FLAG_CONFIG)
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
            val config = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
            if (config) lastHeaders = data
            val s = sender ?: return
            val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
            val flags: UByte = when {
                config -> FLAG_CONFIG
                key -> FLAG_KEY
                else -> 0u
            }
            val advice = s.sendFrame(if (config) configPayload(data) else data, info.presentationTimeUs, flags)
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

    /** `u16 rotation | u16 width | u16 height | headers`, little-endian. */
    private fun configPayload(headers: ByteArray): ByteArray =
        ByteBuffer.allocate(6 + headers.size).order(ByteOrder.LITTLE_ENDIAN).apply {
            putShort(rotation.toShort())
            putShort(width.toShort())
            putShort(height.toShort())
            put(headers)
        }.array()

    fun stop() {
        closed = true
        runCatching { orientation.disable() }
        runCatching { displays.unregisterDisplayListener(displayListener) }
        handler.post {
            closeCamera()
            stopEncoder()
            thread.quitSafely()
        }
    }
}

/** The running camera session (one at a time) and its PC-side events. */
object CameraShare : CameraListener {
    /** Name of the PC using this camera, or null. */
    val usedBy = mutableStateOf<String?>(null)

    @Volatile var capture: CameraCapture? = null
        private set
    @Volatile private var appContext: Context? = null
    @Volatile private var sender: PbScreen? = null
    @Volatile private var ask: CameraAsk? = null
    private val main = Handler(Looper.getMainLooper())
    private val graceEnd = Runnable { stop(userStopped = false) }

    /** Consent and the camera permission are done: open the camera. */
    fun start(ctx: Context, a: CameraAsk): Boolean {
        val node = Bridge.node ?: return false
        appContext = ctx.applicationContext
        val c = CameraCapture(ctx.applicationContext, a)
        val accept = runCatching { c.prepare() }
            .onFailure { Log.e(TAG, "CAMERA prepare failed: $it") }.getOrNull()
        if (accept == null) {
            node.answerCamera(a.id, null)
            c.stop()
            return false
        }
        val s = node.answerCamera(a.id, accept)
        if (s == null) {
            c.stop()
            return false
        }
        node.setCameraListener(this)
        main.removeCallbacks(graceEnd)
        capture?.stop()
        capture = c
        sender = s
        ask = a
        c.attach(s)
        usedBy.value = a.peer
        PbService.instance?.let { notifyCamera(it, a.peer) }
        Log.i(TAG, "CAMERA streaming to ${a.peer}")
        return true
    }

    /** The same PC asked again within the grace: carry on, no prompt. */
    fun resume(id: ULong): Boolean {
        val c = capture ?: return false
        val node = Bridge.node ?: return false
        val s = node.answerCamera(id, c.accept()) ?: return false
        main.removeCallbacks(graceEnd)
        sender = s
        c.attach(s)
        usedBy.value = ask?.peer
        Log.i(TAG, "CAMERA resumed")
        return true
    }

    fun switchLens() {
        val c = capture ?: return
        c.switchTo(1 - c.lens)
    }

    /** The camera could not be opened or was taken by another app. */
    fun cameraFailed() {
        // The session is closed towards the PC as well (it would otherwise
        // keep repeating the last frame for ever), and that PC is declined
        // quietly for a moment instead of being asked again at once.
        main.post { stop(userStopped = true) }
    }

    /** End the session: the camera, the encoder and the service type go. */
    fun stop(userStopped: Boolean) {
        main.removeCallbacks(graceEnd)
        if (userStopped) {
            runCatching { sender?.stop() }
            runCatching { Bridge.node?.cameraUserStopped() }
        }
        capture?.stop()
        capture = null
        sender = null
        usedBy.value = null
        CameraConsentActivity.close()
        // Even if the service is gone, the notification must not stay.
        (PbService.instance ?: appContext)?.let { notifyCamera(it, null) }
        PbService.instance?.leaveCamera()
    }

    override fun onLens(lens: UByte) {
        main.post { capture?.switchTo(lens.toInt().coerceIn(0, 1)) }
    }

    override fun onStopped(resumable: Boolean) {
        // The session that ended: a resume may attach a new one before this
        // runs on the main thread, and that one must not be paused or stopped.
        val ended = sender
        main.post {
            if (sender !== ended) return@post
            if (resumable && capture != null) {
                // The link dropped: keep the camera (frames are not sent) for a resume.
                capture?.detach()
                main.postDelayed(graceEnd, CAMERA_GRACE_MS)
                Log.i(TAG, "CAMERA link lost; waiting for a resume")
            } else {
                stop(userStopped = false)
            }
        }
    }
}
