package dev.phonebridge.app.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class LiteTest {
    private val full = LiteSignals(
        forced = false,
        lowRam = false,
        totalMemBytes = 8L shl 30,
        sdk = 36,
        powerSave = false,
        animatorScale = 1f,
        slowFrames = false,
    )

    @Test
    fun no_signal_means_full_effects() {
        assertNull(liteReason(full))
    }

    @Test
    fun forced_is_reported_first() {
        val s = full.copy(forced = true, lowRam = true, powerSave = true, slowFrames = true)
        assertEquals(LiteReason.Forced, liteReason(s))
    }

    @Test
    fun low_ram_device_turns_lite_on() {
        assertEquals(LiteReason.LowRam, liteReason(full.copy(lowRam = true)))
    }

    @Test
    fun small_memory_turns_lite_on() {
        assertEquals(LiteReason.SmallMemory, liteReason(full.copy(totalMemBytes = 2L shl 30)))
        assertNull("3 GB is enough", liteReason(full.copy(totalMemBytes = 3L shl 30)))
    }

    @Test
    fun old_android_turns_lite_on() {
        assertEquals(LiteReason.OldAndroid, liteReason(full.copy(sdk = 30)))
    }

    @Test
    fun battery_saver_turns_lite_on() {
        assertEquals(LiteReason.BatterySaver, liteReason(full.copy(powerSave = true)))
    }

    @Test
    fun animations_off_turn_lite_on() {
        assertEquals(LiteReason.AnimationsOff, liteReason(full.copy(animatorScale = 0f)))
    }

    @Test
    fun slow_frames_turn_lite_on() {
        assertEquals(LiteReason.SlowFrames, liteReason(full.copy(slowFrames = true)))
    }

    @Test
    fun lite_text_names_the_reason() {
        assertEquals("On: battery saver", liteText(LiteReason.BatterySaver))
        assertEquals("On: animations are off", liteText(LiteReason.AnimationsOff))
        assertEquals("On: this phone has little memory", liteText(LiteReason.SmallMemory))
        assertEquals("Off", liteText(null))
    }
}
