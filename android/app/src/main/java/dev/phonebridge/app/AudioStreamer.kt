package dev.phonebridge.app

import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.MediaRecorder
import android.media.projection.MediaProjection
import android.os.Build
import android.util.Log
import dev.phonebridge.app.audio.AudioReason
import dev.phonebridge.app.audio.AudioSource
import dev.phonebridge.app.audio.SilenceDetector
import dev.phonebridge.app.audio.isSilentPcm16
import dev.phonebridge.app.audio.planCapture
import dev.phonebridge.app.audio.reasonForFailure
import java.util.concurrent.atomic.AtomicBoolean

private const val TAG = "PhoneBridgeAudio"

/** A running source of a share's sound. */
interface AudioCapture {
    fun start(): AudioStartResult
    fun stop()

    /** The longest wait between frames since the last call: above the frame time the reader was late. */
    fun takeMaxFrameGapMs(): Long
}

/** Result of [AudioCapture.start]: null [reason] means capture is running. */
class AudioStartResult(val reason: AudioReason?) {
    val ok: Boolean get() = reason == null
}

/**
 * Captures phone audio as 10 ms frames of raw 16-bit LE PCM (48 kHz stereo by default).
 *
 * The [source] is chosen by the caller and is never substituted:
 *  - [AudioSource.PLAYBACK]: AudioPlaybackCaptureConfiguration. Needs Android 10+, RECORD_AUDIO and a
 *    [MediaProjection]. On Android 14 the projection must be obtained only after the owning service
 *    is in the `mediaProjection` foreground type, and the token is single-use: this class takes
 *    ownership and calls MediaProjection.stop() in [stop]. Never reuse one projection for a second streamer.
 *  - [AudioSource.MICROPHONE]: explicit microphone mode. The owning service must be in the
 *    `microphone` foreground type (FOREGROUND_SERVICE_MICROPHONE) before [start].
 * There is no fallback from one to the other: a playback failure is reported, never recorded as mic.
 *
 * [onFrame] runs on the worker thread with a REUSED buffer: (buffer, length). Copy before returning.
 * [onEnded] runs once on the worker thread if capture ends by itself (read error, onFrame threw,
 * app blocks capture); it is not called after [stop]. Throwing from [onFrame] ends capture.
 */
class AudioStreamer(
    private val ctx: Context,
    private val source: AudioSource,
    private val projection: MediaProjection? = null,
    val sampleRate: Int = 48000,
    val channels: Int = 2,
    private val expectSound: () -> Boolean = {
        (ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager).isMusicActive
    },
    private val onEnded: (AudioReason?) -> Unit = {},
    /** Each frame's verdict, before the silence check: true when it held no sound. */
    private val onLevel: (Boolean) -> Unit = {},
    private val onFrame: (ByteArray, Int) -> Unit,
) : AudioCapture {
    private val isRunning = AtomicBoolean(false)
    private val stopped = AtomicBoolean(false)
    private var workerThread: Thread? = null
    private var audioRecord: AudioRecord? = null
    private val maxFrameGapMs = java.util.concurrent.atomic.AtomicLong(0)

    override fun takeMaxFrameGapMs(): Long = maxFrameGapMs.getAndSet(0)

    @SuppressLint("MissingPermission")
    override fun start(): AudioStartResult {
        if (stopped.get()) return AudioStartResult(AudioReason.UNAVAILABLE) // single use
        if (isRunning.getAndSet(true)) return AudioStartResult(AudioReason.BUSY)

        val hasRecord = ctx.checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
        val refusal = planCapture(source, Build.VERSION.SDK_INT, hasRecord, projection != null, sampleRate, channels)
        if (refusal != null) {
            release()
            return AudioStartResult(refusal)
        }

        val channelConfig = if (channels == 1) AudioFormat.CHANNEL_IN_MONO else AudioFormat.CHANNEL_IN_STEREO
        val minBuf = AudioRecord.getMinBufferSize(sampleRate, channelConfig, AudioFormat.ENCODING_PCM_16BIT)
        val chunkBytes = sampleRate * 2 * channels * FRAME_MS / 1000
        // Room for the reader to stall: a full buffer loses sound and jumps. It only
        // holds audio during a stall, so its size adds no delay otherwise.
        val bufferSize = maxOf(minBuf * 2, sampleRate * 2 * channels * BUFFER_MS / 1000)

        try {
            val rec = when (source) {
                AudioSource.PLAYBACK -> {
                    val config = AudioPlaybackCaptureConfiguration.Builder(projection!!)
                        .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                        .addMatchingUsage(AudioAttributes.USAGE_GAME)
                        .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                        .build()
                    val format = AudioFormat.Builder()
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setSampleRate(sampleRate)
                        .setChannelMask(channelConfig)
                        .build()
                    AudioRecord.Builder()
                        .setAudioPlaybackCaptureConfig(config)
                        .setAudioFormat(format)
                        .setBufferSizeInBytes(bufferSize)
                        .build()
                }
                AudioSource.MICROPHONE -> AudioRecord(
                    MediaRecorder.AudioSource.MIC, sampleRate, channelConfig,
                    AudioFormat.ENCODING_PCM_16BIT, bufferSize,
                )
            }
            audioRecord = rec
            if (rec.state != AudioRecord.STATE_INITIALIZED) {
                Log.e(TAG, "AudioRecord failed to initialize")
                release()
                return AudioStartResult(AudioReason.UNAVAILABLE)
            }
            rec.startRecording()
            if (rec.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                release()
                return AudioStartResult(AudioReason.UNAVAILABLE)
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to start AudioRecord: ${e.message}", e)
            release()
            return AudioStartResult(reasonForFailure(e))
        }

        val rec = audioRecord!!
        val detector = SilenceDetector(FRAME_MS, BLOCKED_AFTER_MS)
        workerThread = Thread({
            val buf = ByteArray(chunkBytes) // reused for every frame
            var filled = 0
            var endReason: AudioReason? = null
            var lastFrameAt = 0L
            var framesSinceCheck = 0
            var soundExpected = false
            Log.i(TAG, "capture started: $source, $sampleRate Hz, $channels ch, $chunkBytes bytes/frame")
            try {
                while (isRunning.get()) {
                    val n = rec.read(buf, filled, chunkBytes - filled)
                    if (n < 0) {
                        Log.e(TAG, "AudioRecord read error $n")
                        endReason = AudioReason.UNAVAILABLE
                        break
                    }
                    filled += n
                    if (filled < chunkBytes) continue
                    filled = 0
                    val now = android.os.SystemClock.elapsedRealtime()
                    if (lastFrameAt != 0L) maxFrameGapMs.accumulateAndGet(now - lastFrameAt, ::maxOf)
                    lastFrameAt = now
                    onLevel(isSilentPcm16(buf, chunkBytes))
                    // A system call: far too slow to make for every frame on this thread.
                    if (++framesSinceCheck >= SOUND_CHECK_FRAMES) {
                        framesSinceCheck = 0
                        soundExpected = expectSound()
                    }
                    if (source == AudioSource.PLAYBACK && detector.onFrame(buf, chunkBytes, soundExpected)) {
                        endReason = AudioReason.APP_BLOCKS_CAPTURE
                        break
                    }
                    onFrame(buf, chunkBytes)
                }
            } catch (e: Exception) {
                Log.e(TAG, "frame delivery failed: ${e.message}")
                endReason = AudioReason.UNAVAILABLE
            }
            if (isRunning.get() && !stopped.get()) {
                // Ended on its own: free the recorder ourselves, then tell the owner once.
                isRunning.set(false)
                cleanup()
                try { onEnded(endReason) } catch (_: Exception) {}
            }
            Log.i(TAG, "capture worker exiting")
        }, "pb-audio-streamer").apply {
            priority = Thread.MAX_PRIORITY
            start()
        }
        return AudioStartResult(null)
    }

    /** Stops capture, joins the worker (unless called from it) and releases AudioRecord and the projection. */
    override fun stop() {
        if (!stopped.compareAndSet(false, true)) return
        isRunning.set(false)
        try { audioRecord?.stop() } catch (e: Exception) { Log.w(TAG, "stop: ${e.message}") }
        val t = workerThread
        if (t != null && t !== Thread.currentThread()) {
            try { t.join(JOIN_MS) } catch (_: InterruptedException) { Thread.currentThread().interrupt() }
            if (t.isAlive) Log.w(TAG, "capture worker did not exit in ${JOIN_MS} ms")
        }
        workerThread = null
        cleanup()
        Log.i(TAG, "AudioStreamer stopped")
    }

    private fun release() {
        stopped.set(true)
        isRunning.set(false)
        cleanup()
    }

    @Synchronized
    private fun cleanup() {
        try { audioRecord?.release() } catch (e: Exception) { Log.w(TAG, "release: ${e.message}") }
        audioRecord = null
        try { projection?.stop() } catch (e: Exception) { Log.w(TAG, "projection stop: ${e.message}") }
    }

    companion object {
        const val FRAME_MS = 10
        /** Long enough that a quiet stretch of a video does not end the share. */
        const val BLOCKED_AFTER_MS = 10_000
        private const val BUFFER_MS = 200
        private const val SOUND_CHECK_FRAMES = 50
        private const val JOIN_MS = 1000L

        @Volatile
        var activeStreamer: AudioStreamer? = null
    }
}
