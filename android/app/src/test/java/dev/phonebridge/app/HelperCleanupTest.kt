package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class HelperCleanupTest {
    @Test
    fun cleanup_runs_in_builds_without_the_helper() {
        assertTrue(helperCleanupDue(helperBuilt = false, done = false))
    }

    @Test
    fun cleanup_runs_only_once() {
        assertFalse(helperCleanupDue(helperBuilt = false, done = true))
    }

    @Test
    fun debug_builds_keep_their_helper() {
        // A developer's own setup stays: debug builds still carry the helper.
        assertFalse(helperCleanupDue(helperBuilt = true, done = false))
    }

    @Test
    fun only_the_helper_prefs_are_cleared() {
        val keys = setOf("audio_helper_paired", "audio_helper_secret", "audio_helper_last_result", "audio_silence_phone", "last_clip")
        assertEquals(listOf("audio_helper_last_result", "audio_helper_paired", "audio_helper_secret"), helperPrefKeys(keys))
    }

    @Test
    fun ask_never_waits_when_the_helper_is_not_offered() {
        assertFalse(helperAskWaits(offered = false, mic = false, paired = true, silence = true))
    }

    @Test
    fun ask_waits_only_for_a_paired_silenced_share() {
        assertTrue(helperAskWaits(offered = true, mic = false, paired = true, silence = true))
        assertFalse(helperAskWaits(offered = true, mic = true, paired = true, silence = true))
        assertFalse(helperAskWaits(offered = true, mic = false, paired = false, silence = true))
        assertFalse(helperAskWaits(offered = true, mic = false, paired = true, silence = false))
    }
}
