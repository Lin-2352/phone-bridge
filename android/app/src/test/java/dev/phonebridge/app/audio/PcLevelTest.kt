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
    fun a_press_moves_away_from_the_held_volume() {
        assertEquals(1, keyStep(held = 1, prev = 1, index = 2))
        assertEquals(-1, keyStep(held = 1, prev = 1, index = 0))
    }

    /** Stepping back is reported too: it must not count as a press the other way. */
    @Test
    fun coming_back_to_the_held_volume_is_not_a_press() {
        assertEquals(0, keyStep(held = 1, prev = 2, index = 1))
        assertEquals(0, keyStep(held = 1, prev = 0, index = 1))
    }

    @Test
    fun a_second_press_before_the_first_was_undone_still_counts() {
        assertEquals(1, keyStep(held = 1, prev = 2, index = 3))
    }

    @Test
    fun a_jump_by_another_app_is_one_step() {
        assertEquals(1, keyStep(held = 1, prev = 1, index = 9))
    }

    @Test
    fun undoing_takes_one_step_toward_the_held_volume() {
        assertEquals(-1, undoStep(held = 1, index = 2))
        assertEquals(1, undoStep(held = 1, index = 0))
        assertEquals(0, undoStep(held = 1, index = 1))
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
