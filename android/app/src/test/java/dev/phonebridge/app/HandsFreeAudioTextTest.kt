package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class HandsFreeAudioTextTest {
    @Test
    fun both_allowed_says_nothing_needs_a_tap() {
        assertEquals("On. The PC can start phone audio with nothing to tap", handsFreeAudioText(true, true))
    }

    @Test
    fun anything_missing_never_claims_it_is_on() {
        for ((over, capture) in listOf(true to false, false to true, false to false)) {
            assertFalse(handsFreeAudioText(over, capture).startsWith("On"))
        }
    }

    @Test
    fun the_capture_prompt_is_named_only_when_it_will_appear() {
        assertTrue(handsFreeAudioText(true, false).contains("capture prompt"))
        assertFalse(handsFreeAudioText(false, true).contains("capture prompt"))
        assertTrue(handsFreeAudioText(false, false).contains("capture prompt"))
    }
}
