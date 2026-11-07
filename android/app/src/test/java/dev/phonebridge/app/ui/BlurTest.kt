package dev.phonebridge.app.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BlurTest {
    @Test
    fun blur_needs_api_31_and_no_lite() {
        assertEquals("below Android 12 a tinted scrim", BlurMode.Scrim, blurMode(30, Modes()))
        assertEquals(BlurMode.Haze, blurMode(31, Modes()))
        assertEquals(BlurMode.Haze, blurMode(36, Modes()))
        assertEquals("lite gives the scrim", BlurMode.Scrim, blurMode(36, Modes(lite = true)))
    }

    @Test
    fun reduced_transparency_is_opaque_on_every_api() {
        for (sdk in listOf(29, 31, 36)) {
            assertEquals("api $sdk", BlurMode.Opaque, blurMode(sdk, Modes(reducedTransparency = true)))
            assertEquals("api $sdk", BlurMode.Opaque, blurMode(sdk, Modes(highContrast = true)))
        }
    }

    @Test
    fun steady_frames_do_not_trip() {
        val budget = FrameBudget(intervalNs = 16_666_667L)
        repeat(600) { assertFalse(budget.onFrame(16_666_667L)) }
    }

    @Test
    fun a_quarter_slow_frames_trip() {
        // 31 of the last 120 frames take twice the interval: more than a quarter, so the budget trips.
        val budget = FrameBudget(intervalNs = 16_666_667L)
        var tripped = false
        repeat(120) { i -> tripped = budget.onFrame(if (i < 31) 33_333_334L else 16_666_667L) }
        assertTrue(tripped)
    }

    @Test
    fun one_spike_does_not_trip() {
        val budget = FrameBudget(intervalNs = 16_666_667L)
        assertFalse(budget.onFrame(200_000_000L))
        repeat(200) { assertFalse(budget.onFrame(16_666_667L)) }
    }
}
