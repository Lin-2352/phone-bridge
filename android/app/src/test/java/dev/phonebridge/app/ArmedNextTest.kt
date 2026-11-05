package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class ArmedNextTest {
    @Test
    fun failed_restart_disarms() {
        assertEquals(ArmedNext.DISARM, armedAfterAnswer(restartOk = false, answered = false))
    }

    @Test
    fun no_session_goes_back_to_idle() {
        // The encoder was restarted for this request: without a session it must stop again.
        assertEquals(ArmedNext.BACK_TO_IDLE, armedAfterAnswer(restartOk = true, answered = false))
    }

    @Test
    fun a_session_shares() {
        assertEquals(ArmedNext.SHARE, armedAfterAnswer(restartOk = true, answered = true))
    }
}
