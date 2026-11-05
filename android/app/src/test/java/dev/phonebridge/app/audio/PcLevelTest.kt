package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PcLevelTest {
    /** With an effect silencing the speaker, the phone's real volume sets the PC's level. */
    @org.junit.Test
    fun the_real_volume_maps_onto_the_pc_level() {
        org.junit.Assert.assertEquals(0, stepForIndex(0, 16))
        org.junit.Assert.assertEquals(PC_LEVEL_STEPS, stepForIndex(16, 16))
        org.junit.Assert.assertEquals(8, stepForIndex(8, 16))
        org.junit.Assert.assertEquals(PC_LEVEL_STEPS, stepForIndex(15, 15))
        org.junit.Assert.assertEquals(1, stepForIndex(1, 25))
    }

    @org.junit.Test
    fun a_strange_volume_reading_never_breaks_the_level() {
        org.junit.Assert.assertEquals(PC_LEVEL_STEPS, stepForIndex(3, 0))
        org.junit.Assert.assertEquals(PC_LEVEL_STEPS, stepForIndex(99, 16))
        org.junit.Assert.assertEquals(0, stepForIndex(-1, 16))
    }

    @Test
    fun the_real_volume_can_show_any_pc_level() {
        assertEquals(9, indexForStep(9, 16))
        assertEquals(0, indexForStep(0, 16))
        assertEquals(15, indexForStep(PC_LEVEL_STEPS, 15))
        assertEquals(13, indexForStep(8, 25))
        assertEquals(0, indexForStep(8, 0))
    }

    @Test
    fun level_stays_within_0_and_16() {
        assertEquals(PC_LEVEL_STEPS, stepAfterKey(PC_LEVEL_STEPS, 1))
        assertEquals(0, stepAfterKey(0, -1))
        assertEquals(9, stepAfterKey(8, 1))
        assertEquals(7, stepAfterKey(8, -1))
        assertEquals(8, stepAfterKey(8, 0))
    }

    private fun frame(vararg samples: Int): ByteArray {
        val b = ByteArray(samples.size * 2)
        samples.forEachIndexed { i, v ->
            b[i * 2] = v.toByte()
            b[i * 2 + 1] = (v shr 8).toByte()
        }
        return b
    }

    private fun sample(b: ByteArray, i: Int): Int =
        (b[i * 2].toInt() and 0xff) or (b[i * 2 + 1].toInt() shl 8)

    @Test
    fun the_top_step_leaves_the_sound_alone_and_step_zero_is_silence() {
        assertEquals(1f, gainForStep(PC_LEVEL_STEPS), 0f)
        assertEquals(0f, gainForStep(0), 0f)
    }

    @Test
    fun each_step_down_is_quieter() {
        for (s in 1..PC_LEVEL_STEPS) assertTrue("step $s", gainForStep(s - 1) < gainForStep(s))
    }

    @Test
    fun steps_out_of_range_are_clamped() {
        assertEquals(PC_LEVEL_STEPS, clampStep(PC_LEVEL_STEPS + 3))
        assertEquals(0, clampStep(-1))
    }

    @Test
    fun full_level_keeps_every_sample_exactly() {
        val l = PcLevel(PC_LEVEL_STEPS)
        val b = frame(1000, -1000, 32767, -32768)
        l.apply(b, b.size, channels = 2)
        assertEquals(listOf(1000, -1000, 32767, -32768), (0..3).map { sample(b, it) })
    }

    @Test
    fun step_zero_ends_in_silence_after_the_fade() {
        val l = PcLevel(PC_LEVEL_STEPS)
        l.step = 0
        val first = frame(*IntArray(960) { 20000 })
        l.apply(first, first.size, channels = 2)
        val second = frame(*IntArray(960) { 20000 })
        l.apply(second, second.size, channels = 2)
        assertEquals(0, sample(second, 0))
        assertEquals(0, sample(second, 959))
    }

    /** A jump within one sample is a click on the PC. */
    @Test
    fun a_level_change_fades_with_no_jump_and_both_channels_stay_equal() {
        val l = PcLevel(PC_LEVEL_STEPS)
        l.step = PC_LEVEL_STEPS / 2
        val b = frame(*IntArray(960) { 20000 })
        l.apply(b, b.size, channels = 2)
        var last = 20000
        for (i in 0 until 480) {
            val left = sample(b, i * 2)
            assertEquals("frame $i", left, sample(b, i * 2 + 1))
            assertTrue("jump at $i: $last to $left", kotlin.math.abs(left - last) <= 200)
            last = left
        }
        assertEquals((20000 * gainForStep(PC_LEVEL_STEPS / 2)).toDouble(), last.toDouble(), 2.0)
    }

    @Test
    fun a_key_press_moves_one_step_and_stops_at_the_ends() {
        val l = PcLevel(PC_LEVEL_STEPS)
        l.adjust(+1)
        assertEquals(PC_LEVEL_STEPS, l.step)
        l.adjust(-1)
        assertEquals(PC_LEVEL_STEPS - 1, l.step)
        repeat(PC_LEVEL_STEPS + 2) { l.adjust(-1) }
        assertEquals(0, l.step)
    }
}
