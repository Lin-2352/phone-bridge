package dev.phonebridge.app

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.util.Log

private const val TAG = "PhoneBridgeAudio"

/**
 * PCM to Opus packets with Android's own encoder, one packet for each 20 ms. Raw stereo needs about
 * 1.5 Mbit/s, and with a video app in front ColorOS caps Relay at 600 kbit/s with a 3000-packet queue
 * behind it, which turned into tens of seconds of delay. This is about 160 kbit/s.
 *
 * Use from one thread; [release] after that thread has stopped.
 */
class OpusEncoder private constructor(private val codec: MediaCodec, private val bytesPerSecond: Long) {
    private val info = MediaCodec.BufferInfo()
    private var writtenBytes = 0L

    /** Feeds PCM and hands every finished packet to [emit], copied. */
    fun encode(pcm: ByteArray, len: Int, emit: (ByteArray) -> Unit) {
        val i = codec.dequeueInputBuffer(INPUT_WAIT_US)
        if (i >= 0) {
            val b = codec.getInputBuffer(i) ?: return
            b.clear()
            b.put(pcm, 0, minOf(len, b.remaining()))
            codec.queueInputBuffer(i, 0, b.position(), writtenBytes * 1_000_000 / bytesPerSecond, 0)
            writtenBytes += len
        }
        while (true) {
            val o = codec.dequeueOutputBuffer(info, 0)
            if (o < 0) {
                if (o == MediaCodec.INFO_TRY_AGAIN_LATER) return
                continue // format or buffers changed
            }
            // The first buffer is the stream header, not audio.
            if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0 && info.size > 0) {
                val out = codec.getOutputBuffer(o)
                if (out != null) {
                    val packet = ByteArray(info.size)
                    out.position(info.offset)
                    out.get(packet)
                    emit(packet)
                }
            }
            codec.releaseOutputBuffer(o, false)
        }
    }

    fun release() {
        runCatching { codec.stop() }
        runCatching { codec.release() }
    }

    companion object {
        private const val BIT_RATE = 160_000
        private const val INPUT_WAIT_US = 5_000L

        /** Null when this phone cannot encode Opus; the caller then sends PCM. */
        fun create(sampleRate: Int, channels: Int): OpusEncoder? {
            if (sampleRate != 48_000) return null
            return try {
                val f = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_OPUS, sampleRate, channels)
                f.setInteger(MediaFormat.KEY_BIT_RATE, BIT_RATE)
                f.setInteger(MediaFormat.KEY_PCM_ENCODING, android.media.AudioFormat.ENCODING_PCM_16BIT)
                f.setInteger(MediaFormat.KEY_BITRATE_MODE, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR)
                val c = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_OPUS)
                c.configure(f, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                c.start()
                OpusEncoder(c, sampleRate.toLong() * channels * 2)
            } catch (e: Exception) {
                Log.w(TAG, "AUDIO no Opus encoder, sending PCM: $e")
                null
            }
        }
    }
}
