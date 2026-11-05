package dev.phonebridge.app.audio

import android.view.KeyEvent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The opt-in key filter (Relay control): with YouTube or Instagram in front, Android sends the volume keys to
 * that app's own player, so only an accessibility key filter can give them to the PC's level.
 */
class VolumeKeyFilterTest {
    @Test
    fun the_keys_are_held_only_with_the_setting_on_during_a_silenced_share() {
        assertTrue(holdsVolumeKeys(setting = true, silencedShare = true))
        assertFalse(holdsVolumeKeys(setting = false, silencedShare = true))
        assertFalse(holdsVolumeKeys(setting = true, silencedShare = false))
    }

    @Test
    fun a_press_moves_the_pc_level_one_step_its_way() {
        assertEquals(1, volumeKeyStep(true, KeyEvent.KEYCODE_VOLUME_UP, KeyEvent.ACTION_DOWN))
        assertEquals(-1, volumeKeyStep(true, KeyEvent.KEYCODE_VOLUME_DOWN, KeyEvent.ACTION_DOWN))
    }

    @Test
    fun letting_go_is_swallowed_too_so_the_app_in_front_never_sees_half_a_press() {
        assertEquals(0, volumeKeyStep(true, KeyEvent.KEYCODE_VOLUME_UP, KeyEvent.ACTION_UP))
    }

    @Test
    fun other_keys_and_a_released_hold_pass_through() {
        assertEquals(null, volumeKeyStep(true, KeyEvent.KEYCODE_BACK, KeyEvent.ACTION_DOWN))
        assertEquals(null, volumeKeyStep(false, KeyEvent.KEYCODE_VOLUME_UP, KeyEvent.ACTION_DOWN))
    }

    @Test
    fun mute_key_is_left_to_the_phone() {
        assertEquals(null, volumeKeyStep(true, KeyEvent.KEYCODE_VOLUME_MUTE, KeyEvent.ACTION_DOWN))
    }
}
