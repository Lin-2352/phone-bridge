package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class FocusPlanTest {
    @Test
    fun focus_codes_map_to_events() {
        // AudioManager.AUDIOFOCUS_GAIN, _LOSS, _LOSS_TRANSIENT, _LOSS_TRANSIENT_CAN_DUCK
        assertEquals(FocusEvent.GAIN, focusEventOf(1))
        assertEquals(FocusEvent.LOSS, focusEventOf(-1))
        assertEquals(FocusEvent.LOSS_TRANSIENT, focusEventOf(-2))
        assertEquals(FocusEvent.LOSS_CAN_DUCK, focusEventOf(-3))
    }

    @Test
    fun unknown_focus_code_is_ignored() {
        assertNull(focusEventOf(0))
        assertNull(focusEventOf(7))
    }

    @Test
    fun a_call_pauses_and_gain_plays_again() {
        assertEquals(PlayerAction.Pause, playerReaction(FocusEvent.LOSS_TRANSIENT))
        assertEquals(PlayerAction.Play, playerReaction(FocusEvent.GAIN))
    }

    @Test
    fun another_app_taking_over_ends_playback() {
        assertEquals(PlayerAction.End, playerReaction(FocusEvent.LOSS))
    }

    @Test
    fun a_notification_ducks() {
        assertEquals(PlayerAction.Duck(0.2f), playerReaction(FocusEvent.LOSS_CAN_DUCK))
    }

    @Test
    fun unplugged_headphones_end_playback() {
        // Never blast PC audio from the speaker.
        assertEquals(PlayerAction.End, playerReaction(FocusEvent.NOISY))
    }
}
