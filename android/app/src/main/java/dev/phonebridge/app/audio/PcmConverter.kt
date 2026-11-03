package dev.phonebridge.app.audio

/**
 * Streaming converter from 16-bit LE PCM (mono or stereo, 44.1 or 48 kHz) to 48 kHz stereo shorts.
 * Linear interpolation; the fractional position and last frame carry across calls so chunk
 * borders are seamless. Not thread-safe (one network thread); allocation-free in [convert].
 */
class PcmConverter(private val inRate: Int, private val inChannels: Int) {
    private var prevL = 0
    private var prevR = 0
    private var pos = 0.0 // input index of the next output frame, in the current chunk's coordinates
    private val step = inRate.toDouble() / OUT_RATE

    /** Converts [len] bytes of [src] into [out]; returns shorts written (even). [out] must hold [maxOutShorts]. */
    fun convert(src: ByteArray, len: Int, out: ShortArray): Int {
        val frameBytes = 2 * inChannels
        val frames = len / frameBytes
        if (frames == 0) return 0
        var o = 0
        if (inRate == OUT_RATE) {
            for (f in 0 until frames) {
                val b = f * frameBytes
                val l = sample(src, b)
                val r = if (inChannels == 2) sample(src, b + 2) else l
                out[o++] = l.toShort()
                out[o++] = r.toShort()
            }
            return o
        }
        // Input index i in [0, frames) is new data; index -1 is the carried previous frame.
        // p is the input index of the next output frame, in (-1, frames-1] here.
        var p = pos
        while (p <= frames - 1) {
            val i0 = Math.floor(p).toInt()
            val t = p - i0
            val l0: Int; val r0: Int
            if (i0 < 0) { l0 = prevL; r0 = prevR } else {
                val b = i0 * frameBytes
                l0 = sample(src, b)
                r0 = if (inChannels == 2) sample(src, b + 2) else l0
            }
            val b1 = (i0 + 1) * frameBytes
            val l1: Int; val r1: Int
            if (i0 + 1 >= frames) { l1 = l0; r1 = r0 } else {
                l1 = sample(src, b1)
                r1 = if (inChannels == 2) sample(src, b1 + 2) else l1
            }
            out[o++] = (l0 + (l1 - l0) * t).toInt().toShort()
            out[o++] = (r0 + (r1 - r0) * t).toInt().toShort()
            p += step
        }
        val last = (frames - 1) * frameBytes
        prevL = sample(src, last)
        prevR = if (inChannels == 2) sample(src, last + 2) else prevL
        pos = p - frames // same position in the next chunk's coordinates
        return o
    }

    private fun sample(b: ByteArray, i: Int): Int = (b[i].toInt() and 0xff) or (b[i + 1].toInt() shl 8)

    companion object {
        const val OUT_RATE = 48000

        /** Upper bound of shorts [convert] writes for [inBytes] bytes of input. */
        fun maxOutShorts(inRate: Int, inChannels: Int, inBytes: Int): Int {
            val frames = inBytes / (2 * inChannels)
            return ((frames.toLong() * OUT_RATE / inRate).toInt() + 2) * 2
        }
    }
}
