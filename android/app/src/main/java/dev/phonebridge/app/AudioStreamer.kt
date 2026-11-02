package dev.phonebridge.app

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.MediaRecorder
import android.media.projection.MediaProjection
import android.os.Build
import android.util.Log
import java.io.OutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean

private const val TAG = "PhoneBridgeAudio"

/**
 * Captures internal media audio using [AudioPlaybackCaptureConfiguration] on API 29+
 * (or fallback to microphone input) and streams raw 16-bit PCM (interleaved stereo, 48 kHz)
 * with 4-byte LE frame length prefixes.
 */
class AudioStreamer(
    private val ctx: Context,
    private val projection: MediaProjection? = null,
    val sampleRate: Int = 48000,
    val channels: Int = 2,
    private val onFrame: ((ByteArray) -> Unit)? = null,
) {
    private val isRunning = AtomicBoolean(false)
    private var workerThread: Thread? = null
    private var audioRecord: AudioRecord? = null

    @SuppressLint("MissingPermission")
    fun start(): Boolean {
        if (isRunning.getAndSet(true)) {
            Log.w(TAG, "AudioStreamer already running")
            return true
        }

        val channelConfig = if (channels == 1) {
            AudioFormat.CHANNEL_IN_MONO
        } else {
            AudioFormat.CHANNEL_IN_STEREO
        }

        val minBufSize = AudioRecord.getMinBufferSize(
            sampleRate,
            channelConfig,
            AudioFormat.ENCODING_PCM_16BIT
        )

        val bufferSize = (minBufSize * 2).coerceAtLeast(4096)

        try {
            audioRecord = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && projection != null) {
                // Internal playback audio capture (music, media, apps)
                val config = AudioPlaybackCaptureConfiguration.Builder(projection)
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
            } else {
                // Fallback to standard microphone / voice communication
                AudioRecord(
                    MediaRecorder.AudioSource.MIC,
                    sampleRate,
                    channelConfig,
                    AudioFormat.ENCODING_PCM_16BIT,
                    bufferSize
                )
            }

            if (audioRecord?.state != AudioRecord.STATE_INITIALIZED) {
                Log.e(TAG, "AudioRecord failed to initialize")
                stop()
                return false
            }

            audioRecord?.startRecording()
        } catch (e: Exception) {
            Log.e(TAG, "Failed to start AudioRecord: ${e.message}", e)
            stop()
            return false
        }

        workerThread = Thread({
            // 20ms of audio per chunk for low latency:
            // 48000 samples/sec * 2 bytes/sample * channels * 0.02 sec
            val chunkBytes = (sampleRate * 2 * channels * 20) / 1000
            val readBuf = ByteArray(chunkBytes)

            Log.i(TAG, "Audio recording started (chunk: $chunkBytes bytes, rate: $sampleRate, channels: $channels)")

            while (isRunning.get()) {
                val read = audioRecord?.read(readBuf, 0, readBuf.size) ?: -1
                if (read > 0) {
                    val frame = if (read == readBuf.size) {
                        readBuf.copyOf()
                    } else {
                        readBuf.copyOf(read)
                    }
                    try {
                        onFrame?.invoke(frame)
                    } catch (e: Exception) {
                        Log.e(TAG, "Error delivering audio frame: ${e.message}")
                        break
                    }
                } else if (read < 0) {
                    Log.e(TAG, "AudioRecord error read code: $read")
                    break
                }
            }
            Log.i(TAG, "Audio recording worker exiting")
        }, "pb-audio-streamer").apply {
            priority = Thread.MAX_PRIORITY
            start()
        }

        return true
    }

    fun stop() {
        if (!isRunning.getAndSet(false)) return
        try {
            audioRecord?.stop()
        } catch (e: Exception) {
            Log.w(TAG, "Error stopping AudioRecord: ${e.message}")
        }
        try {
            audioRecord?.release()
        } catch (e: Exception) {
            Log.w(TAG, "Error releasing AudioRecord: ${e.message}")
        }
        audioRecord = null

        workerThread?.interrupt()
        workerThread = null
        Log.i(TAG, "AudioStreamer stopped")
    }

    companion object {
        @Volatile
        var activeStreamer: AudioStreamer? = null
    }
}
