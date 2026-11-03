package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SpeakerGuardTest {
    private fun guard() = SpeakerGuard(frameMs = 10, silentLimitMs = 100, checkMs = 50)

    private fun SpeakerGuard.silence(frames: Int, expected: Boolean = true): SpeakerAction {
        var last = SpeakerAction.NONE
        repeat(frames) { last = onFrame(silent = true, soundExpected = expected) }
        return last
    }

    @Test
    fun long_silence_while_sound_plays_starts_a_check() {
        val g = guard()
        assertEquals(SpeakerAction.NONE, g.silence(9))
        assertEquals(SpeakerAction.UNMUTE_TO_CHECK, g.silence(1))
        assertTrue(g.isChecking)
    }

    @Test
    fun sound_returning_when_unmuted_proves_the_mute_killed_the_capture() {
        val g = guard()
        g.silence(10)
        assertEquals(SpeakerAction.GIVE_UP, g.onFrame(silent = false, soundExpected = true))
    }

    @Test
    fun a_quiet_video_is_not_a_dead_capture_so_it_mutes_again() {
        val g = guard()
        g.silence(10)
        assertEquals(SpeakerAction.NONE, g.silence(4))
        assertEquals(SpeakerAction.MUTE_AGAIN, g.silence(1))
        assertFalse(g.isChecking)
    }

    @Test
    fun a_paused_player_is_never_checked() {
        val g = guard()
        assertEquals(SpeakerAction.NONE, g.silence(1000, expected = false))
    }

    private fun settling() = SpeakerGuard(frameMs = 10, silentLimitMs = 100, checkMs = 50, settleMs = 50)

    @Test
    fun sound_heard_while_muted_proves_the_capture_survives_so_it_never_unmutes() {
        val g = settling()
        repeat(6) { assertEquals(SpeakerAction.NONE, g.onFrame(silent = false, soundExpected = true)) }
        assertTrue(g.proven)
        assertEquals(SpeakerAction.NONE, g.silence(1000))
        assertFalse(g.isChecking)
    }

    @Test
    fun sound_right_after_muting_proves_nothing_since_it_was_captured_before_the_mute() {
        val g = settling()
        repeat(2) { g.onFrame(silent = false, soundExpected = true) }
        assertFalse(g.proven)
        assertEquals(SpeakerAction.UNMUTE_TO_CHECK, g.silence(10))
    }

    @Test
    fun a_phone_already_proven_never_checks() {
        val g = SpeakerGuard(frameMs = 10, silentLimitMs = 100, checkMs = 50, proven = true)
        assertEquals(SpeakerAction.NONE, g.silence(1000))
    }

    @Test
    fun any_sound_restarts_the_count() {
        val g = guard()
        g.silence(9)
        assertEquals(SpeakerAction.NONE, g.onFrame(silent = false, soundExpected = true))
        assertEquals(SpeakerAction.NONE, g.silence(9))
    }
}
