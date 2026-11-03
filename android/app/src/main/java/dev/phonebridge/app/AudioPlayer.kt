package dev.phonebridge.app

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.util.Log
import dev.phonebridge.app.audio.JitterBuffer
import dev.phonebridge.app.audio.PcmConverter

private const val TAG = "PhoneBridgeAudio"

/**
 * Plays PC audio (raw 16-bit LE PCM frames, mono or stereo, 44.1 or 48 kHz) on the phone.
 *
 * Frames go through [PcmConverter] to 48 kHz stereo and into a [JitterBuffer] (target 120 ms,
 * drops the oldest above 300 ms, silence on underrun). A writer thread feeds a low-latency
 * USAGE_MEDIA AudioTrack in 20 ms blocks.
 *
 * Threading: [push] is for ONE producer thread (the network receive loop). [start], [stop] and
 * [release] are safe from any thread, in any order, and idempotent; [push] after [stop] is ignored.
 * A player is single-use: create a new one per session.
 */
class AudioPlayer(sampleRate: Int, channels: Int) {
    private val converter = PcmConverter(sampleRate, channels)
    private val jitter = JitterBuffer(OUT_RATE, 2, TARGET_MS, MAX_MS)
    private var convBuf = ShortArray(PcmConverter.maxOutShorts(sampleRate, channels, 65536))
    private val lock = Any()
    private var track: AudioTrack? = null
    private var worker: Thread? = null
    @Volatile private var running = false
    @Volatile private var dead = false

    /** Packets of audio dropped because the buffer was over its maximum (milliseconds). */
    val droppedMs: Long get() = jitter.droppedMs

    /** Times the writer ran out of audio and played silence. */
    val underruns: Long get() = jitter.underruns

    val bufferedMs: Int get() = jitter.bufferedMs()

    /** Creates the AudioTrack and starts the writer. Returns false if the device refuses the track. */
    fun start(): Boolean = synchronized(lock) {
        if (dead) return false
        if (running) return true
        try {
            val minBuf = AudioTrack.getMinBufferSize(OUT_RATE, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_16BIT)
            if (minBuf <= 0) return false
            val t = AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build(),
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setSampleRate(OUT_RATE)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
                        .build(),
                )
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
                .setBufferSizeInBytes(maxOf(minBuf, BLOCK_SHORTS * 2 * 2))
                .build()
            if (t.state != AudioTrack.STATE_INITIALIZED) {
                t.release()
                return false
            }
            track = t
            running = true
            t.play()
            worker = Thread({ writeLoop(t) }, "pb-audio-player").apply {
                priority = Thread.MAX_PRIORITY
                start()
            }
            true
        } catch (e: Exception) {
            Log.e(TAG, "AudioTrack start failed: ${e.message}", e)
            running = false
            track?.release()
            track = null
            false
        }
    }

    /** Queues one PCM frame (as received). Never blocks; ignores frames that are too long or after stop. */
    fun push(frame: ByteArray, len: Int = frame.size) {
        if (!running || len <= 0) return
        if (PcmConverter.maxOutShorts(srcRate, srcChannels, len) > convBuf.size) return
        val n = converter.convert(frame, len, convBuf)
        jitter.push(convBuf, n)
    }

    private val srcRate = sampleRate
    private val srcChannels = channels

    private fun writeLoop(t: AudioTrack) {
        val block = ShortArray(BLOCK_SHORTS)
        while (running) {
            jitter.read(block, BLOCK_SHORTS) // fills silence when priming or underrun
            val w = try {
                t.write(block, 0, BLOCK_SHORTS) // blocking write paces the loop
            } catch (_: Exception) {
                -1
            }
            if (w < 0) break
        }
    }

    /** Stops playback and frees the AudioTrack; joins the writer. Safe from any thread, idempotent. */
    fun stop() = release()

    fun release() {
        val t: AudioTrack?
        val w: Thread?
        synchronized(lock) {
            dead = true
            running = false
            t = track
            w = worker
            track = null
            worker = null
        }
        try { t?.pause() } catch (_: Exception) {}
        try { t?.flush() } catch (_: Exception) {}
        if (w != null && w !== Thread.currentThread()) {
            try { w.join(1000) } catch (_: InterruptedException) { Thread.currentThread().interrupt() }
        }
        try { t?.release() } catch (_: Exception) {}
        jitter.clear()
    }

    private companion object {
        const val OUT_RATE = 48000
        const val TARGET_MS = 120
        const val MAX_MS = 300
        const val BLOCK_SHORTS = OUT_RATE * 2 * 20 / 1000
    }
}
