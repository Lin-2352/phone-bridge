package dev.phonebridge.app.audio

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SpeakerGuardTest {
    @Test
    fun silence_while_sound_plays_proves_the_capture_died() {
        val g = SpeakerGuard(frameMs = 10, giveUpAfterMs = 100)
        repeat(9) { assertFalse(g.onFrame(silent = true, soundExpected = true)) }
        assertTrue(g.onFrame(silent = true, soundExpected = true))
        assertTrue("the verdict stays", g.onFrame(silent = false, soundExpected = true))
    }

    @Test
    fun a_paused_player_is_not_a_dead_capture() {
        val g = SpeakerGuard(frameMs = 10, giveUpAfterMs = 100)
        repeat(1000) { assertFalse(g.onFrame(silent = true, soundExpected = false)) }
    }

    @Test
    fun any_sound_restarts_the_count() {
        val g = SpeakerGuard(frameMs = 10, giveUpAfterMs = 100)
        repeat(9) { g.onFrame(silent = true, soundExpected = true) }
        assertFalse(g.onFrame(silent = false, soundExpected = true))
        repeat(9) { assertFalse(g.onFrame(silent = true, soundExpected = true)) }
    }
}
