package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NoSoundHintTest {
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

    /**
     * Measured on 2026-11-05: YouTube's home screen opened a track that stayed started and silent while
     * its session read stopped, with no capture block in its flags. That ended the share as "the app
     * does not allow capture", put the speaker back at 14 and left nothing to test.
     */
    @Test
    fun a_share_that_starts_quiet_with_a_silent_player_open_only_ever_gets_a_hint() {
        val h = NoSoundHint(afterMs = 10_000)
        for (t in 0L until 10_000L step 1000) {
            assertFalse(h.onTick(t, soundCaptured = false, playerActive = true))
            assertFalse(h.showing)
        }
        assertTrue(h.onTick(10_000, soundCaptured = false, playerActive = true))
        assertTrue(h.showing)
        // An hour on it has said so once, and that is all it can ever do.
        for (t in 11_000L..3_600_000L step 1000) assertFalse(h.onTick(t, soundCaptured = false, playerActive = true))
        assertTrue(h.showing)
    }

    @Test
    fun sound_takes_the_hint_away_at_once() {
        val h = NoSoundHint(afterMs = 10_000)
        h.onTick(0, soundCaptured = false, playerActive = true)
        h.onTick(10_000, soundCaptured = false, playerActive = true)
        assertTrue(h.onTick(11_000, soundCaptured = true, playerActive = true))
        assertFalse(h.showing)
    }

    @Test
    fun a_quiet_stretch_after_sound_counts_from_the_start_again() {
        val h = NoSoundHint(afterMs = 10_000)
        h.onTick(0, soundCaptured = true, playerActive = true)
        assertFalse(h.onTick(1_000, soundCaptured = false, playerActive = true))
        assertFalse(h.onTick(10_999, soundCaptured = false, playerActive = true))
        assertTrue(h.onTick(11_000, soundCaptured = false, playerActive = true))
    }

    @Test
    fun nothing_playing_never_gets_a_hint() {
        val h = NoSoundHint(afterMs = 10_000)
        for (t in 0L..600_000L step 1000) assertFalse(h.onTick(t, soundCaptured = false, playerActive = false))
        assertFalse(h.showing)
    }

    @Test
    fun the_player_closing_takes_the_hint_away_and_restarts_the_count() {
        val h = NoSoundHint(afterMs = 10_000)
        h.onTick(0, soundCaptured = false, playerActive = true)
        h.onTick(10_000, soundCaptured = false, playerActive = true)
        assertTrue(h.onTick(11_000, soundCaptured = false, playerActive = false))
        assertFalse(h.onTick(12_000, soundCaptured = false, playerActive = true))
        assertFalse(h.showing)
    }

    @Test
    fun the_session_says_where_the_sound_goes_and_when_none_is_reaching_it() {
        assertEquals("Sent to DESK", sessionDetail(AudioDirection.PHONE_TO_PC, "DESK", noSound = false))
        assertEquals("From DESK", sessionDetail(AudioDirection.PC_TO_PHONE, "DESK", noSound = false))
        val quiet = sessionDetail(AudioDirection.PHONE_TO_PC, "DESK", noSound = true)
        assertTrue(quiet, quiet.startsWith("Nothing to send to DESK right now."))
        assertTrue(quiet, quiet.contains("Some apps don't allow"))
    }
}
