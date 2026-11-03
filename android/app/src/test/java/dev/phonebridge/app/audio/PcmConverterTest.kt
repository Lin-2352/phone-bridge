package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PcmConverterTest {
    private fun bytes(vararg s: Int): ByteArray {
        val b = ByteArray(s.size * 2)
        for (i in s.indices) {
            b[i * 2] = (s[i] and 0xff).toByte()
            b[i * 2 + 1] = (s[i] shr 8).toByte()
        }
        return b
    }

    @Test
    fun stereoPassesThroughAt48k() {
        val c = PcmConverter(48000, 2)
        val src = bytes(1, -2, 300, -400)
        val out = ShortArray(16)
        assertEquals(4, c.convert(src, src.size, out))
        assertEquals(listOf<Short>(1, -2, 300, -400), out.take(4))
    }

    @Test
    fun monoIsDuplicatedToBothChannels() {
        val c = PcmConverter(48000, 1)
        val src = bytes(10, -20, 30)
        val out = ShortArray(16)
        assertEquals(6, c.convert(src, src.size, out))
        assertEquals(listOf<Short>(10, 10, -20, -20, 30, 30), out.take(6))
    }

    @Test
    fun lengthRatioFor44100To48000() {
        val c = PcmConverter(44100, 2)
        val src = ByteArray(882 * 4) // 20 ms
        val out = ShortArray(2000)
        var total = 0
        repeat(50) { total += c.convert(src, src.size, out) / 2 } // 1 s
        assertTrue("got $total", total in 47990..48010)
    }

    @Test
    fun rampStaysMonotonicAcrossChunks() {
        val c = PcmConverter(44100, 1)
        val out = ShortArray(4000)
        var prev = Short.MIN_VALUE
        var n = 0
        for (chunk in 0 until 10) {
            val src = ByteArray(441 * 2)
            for (i in 0 until 441) {
                val v = (chunk * 441 + i) * 4
                src[i * 2] = (v and 0xff).toByte()
                src[i * 2 + 1] = (v shr 8).toByte()
            }
            val got = c.convert(src, src.size, out)
            for (i in 0 until got step 2) {
                assertTrue("not monotonic at $n", out[i] >= prev)
                assertEquals(out[i], out[i + 1])
                prev = out[i]
                n++
            }
        }
        assertTrue("n=$n", n > 4000)
    }

    @Test
    fun maxOutShortsBoundsTheOutputBuffer() {
        // 20 ms of 44.1 kHz mono -> at most 960 stereo frames + slack
        val bound = PcmConverter.maxOutShorts(44100, 1, 882 * 2)
        assertTrue(bound >= 960 * 2)
        assertTrue(bound <= 970 * 2)
    }
}
