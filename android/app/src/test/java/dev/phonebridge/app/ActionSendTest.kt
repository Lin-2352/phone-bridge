package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ActionSendTest {
    @Test
    fun no_options_below_34() {
        assertNull(bgStartMode(33))
        assertNull(bgStartMode(29))
    }

    @Test
    fun allowed_on_34_and_35() {
        assertEquals(1, bgStartMode(34)) // MODE_BACKGROUND_ACTIVITY_START_ALLOWED
        assertEquals(1, bgStartMode(35))
    }

    @Test
    fun allow_always_from_36() {
        assertEquals(3, bgStartMode(36)) // MODE_BACKGROUND_ACTIVITY_START_ALLOW_ALWAYS
    }

    @Test
    fun open_needs_a_tap_when_relay_cannot_start_activities() {
        assertEquals(OpenOutcome.TAP_ON_PHONE, openOutcome(canStartActivity = false))
        assertEquals(OpenOutcome.SEND, openOutcome(canStartActivity = true))
    }

    @Test
    fun only_a_live_control_session_or_relays_screen_lets_the_pc_open_apps() {
        // "Keep Control on" leaves the service bound between sessions: that alone must not count.
        assertEquals(false, relayMayStartActivity(overlay = false, controlSession = false, uiVisible = false))
        assertEquals(true, relayMayStartActivity(overlay = false, controlSession = true, uiVisible = false))
        assertEquals(true, relayMayStartActivity(overlay = false, controlSession = false, uiVisible = true))
        assertEquals(true, relayMayStartActivity(overlay = true, controlSession = false, uiVisible = false))
    }
}
