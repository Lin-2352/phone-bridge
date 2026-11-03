package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class JitterBufferTest {
    // 48 kHz stereo: 1920 shorts per 20 ms
    private fun frame(v: Short, ms: Int = 20) = ShortArray(48 * ms * 2) { v }
    private fun jb() = JitterBuffer(48000, 2, targetMs = 100, maxMs = 300)

    @Test
    fun silenceUntilPrebuffered() {
        val b = jb()
        val out = ShortArray(1920) { 7 }
        b.push(frame(5), 1920)
        assertEquals(0, b.read(out, 1920))
        assertTrue(out.all { it == 0.toShort() })
        assertEquals(0L, b.underruns) // still priming, not an underrun
    }

    @Test
    fun playsInOrderOnceTargetIsReached() {
        val b = jb()
        for (i in 1..5) b.push(frame(i.toShort()), 1920) // 100 ms
        val out = ShortArray(1920)
        for (i in 1..5) {
            assertEquals(1920, b.read(out, 1920))
            assertTrue(out.all { it == i.toShort() })
        }
    }

    @Test
    fun underrunGivesSilenceCountsAndReprimes() {
        val b = jb()
        for (i in 1..5) b.push(frame(9), 1920)
        val out = ShortArray(1920)
        repeat(5) { b.read(out, 1920) }
        assertEquals(0, b.read(out, 1920))
        assertTrue(out.all { it == 0.toShort() })
        assertEquals(1L, b.underruns)
        b.push(frame(3), 1920) // one frame is below target: still silence
        assertEquals(0, b.read(out, 1920))
        assertEquals(1L, b.underruns)
    }

    @Test
    fun partialReadIsPaddedWithSilence() {
        val b = jb()
        b.push(frame(4, 100), 9600) // 100 ms
        val out = ShortArray(9600 + 1920) { 1 }
        assertEquals(9600, b.read(out, out.size))
        assertEquals(4.toShort(), out[9599])
        assertEquals(0.toShort(), out[9600])
        assertEquals(1L, b.underruns)
    }

    @Test
    fun overflowDropsOldestDownToTarget() {
        val b = jb()
        for (i in 1..20) b.push(frame(i.toShort()), 1920) // 400 ms > 300 ms
        assertTrue(b.bufferedMs() in 100..300)
        assertTrue(b.droppedMs > 0)
        val out = ShortArray(1920)
        b.read(out, 1920)
        assertTrue("oldest dropped, got ${out[0]}", out[0] > 5)
        var last: Short = 0
        while (b.read(out, 1920) > 0) last = out[0]
        assertEquals(20.toShort(), last) // newest survives
    }

    @Test
    fun oversizedPushKeepsTheNewestAndNeverGrows() {
        val b = jb()
        val big = ShortArray(48 * 2 * 1000) { (it / 96).toShort() } // 1 s ramp, one step per ms
        b.push(big, big.size)
        assertTrue(b.bufferedMs() <= 300)
        assertTrue(b.droppedMs >= 700)
    }

    @Test
    fun clearEmptiesAndReprimes() {
        val b = jb()
        for (i in 1..5) b.push(frame(2), 1920)
        b.clear()
        assertEquals(0, b.bufferedMs())
        assertEquals(0, b.read(ShortArray(1920), 1920))
    }
}
