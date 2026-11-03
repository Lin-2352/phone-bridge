package dev.phonebridge.app.audio

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SilenceDetectorTest {
    private val silent = ByteArray(3840)
    private val loud = ByteArray(3840).also { it[101] = 0x40 } // one sample far above the floor

    @Test
    fun silentBytesAreSilent() {
        assertTrue(isSilentPcm16(silent, silent.size))
        assertFalse(isSilentPcm16(loud, loud.size))
    }

    @Test
    fun respectsTheLengthArgument() {
        assertTrue(isSilentPcm16(loud, 50)) // the loud sample is beyond byte 50
    }

    @Test
    fun blocksOnlyAfterTwoSecondsOfSilenceWhileSoundIsExpected() {
        val d = SilenceDetector(20, 2000)
        repeat(99) { assertFalse(d.onFrame(silent, silent.size, true)) }
        assertFalse(d.blocked)
        assertTrue(d.onFrame(silent, silent.size, true)) // 100th frame = 2.0 s
        assertTrue(d.blocked)
    }

    @Test
    fun soundResetsTheCount() {
        val d = SilenceDetector(20, 2000)
        repeat(99) { d.onFrame(silent, silent.size, true) }
        assertFalse(d.onFrame(loud, loud.size, true))
        repeat(99) { assertFalse(d.onFrame(silent, silent.size, true)) }
    }

    @Test
    fun silenceWhenNothingIsPlayingNeverBlocks() {
        val d = SilenceDetector(20, 2000)
        repeat(1000) { assertFalse(d.onFrame(silent, silent.size, false)) }
        repeat(99) { assertFalse(d.onFrame(silent, silent.size, true)) }
    }

    @Test
    fun resetClearsBlocked() {
        val d = SilenceDetector(20, 2000)
        repeat(100) { d.onFrame(silent, silent.size, true) }
        d.reset()
        assertFalse(d.blocked)
        assertFalse(d.onFrame(silent, silent.size, true))
    }
}
