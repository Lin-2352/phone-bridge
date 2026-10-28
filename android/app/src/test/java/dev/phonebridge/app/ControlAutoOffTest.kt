package dev.phonebridge.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ControlAutoOffTest {
    @Test
    fun switches_off_once_a_session_has_run_and_nothing_is_active() {
        assertTrue(shouldSwitchOff(sessionSeen = true, controlledBy = null, sharingWith = null))
    }

    @Test
    fun stays_on_after_the_user_just_turned_it_on() {
        // No session yet: switching off now would undo the user's own tap.
        assertFalse(shouldSwitchOff(sessionSeen = false, controlledBy = null, sharingWith = null))
    }

    @Test
    fun stays_on_while_the_pc_still_controls_the_phone() {
        assertFalse(shouldSwitchOff(sessionSeen = true, controlledBy = "DeskTray", sharingWith = null))
    }

    @Test
    fun stays_on_while_the_screen_is_still_shared() {
        // The phone-screen window taps through the same service.
        assertFalse(shouldSwitchOff(sessionSeen = true, controlledBy = null, sharingWith = "DeskTray"))
    }
}
