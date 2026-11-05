package dev.phonebridge.app.ui.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RemoteLinkTest {
    @Test fun a_refused_send_while_connected_means_the_link_is_lost() {
        assertEquals(LinkPhase.Lost, linkAfterSend(LinkPhase.Connected, sent = false))
    }

    @Test fun a_good_send_changes_nothing() {
        assertEquals(LinkPhase.Connected, linkAfterSend(LinkPhase.Connected, sent = true))
    }

    @Test fun a_refused_send_does_not_override_other_phases() {
        // Connecting or already failed or lost: a stray false (a late gesture) must not rewrite the state.
        for (p in listOf(LinkPhase.Connecting, LinkPhase.Lost, LinkPhase.Failed, LinkPhase.Off)) {
            assertEquals(p, linkAfterSend(p, sent = false))
        }
    }

    @Test fun retry_only_when_something_failed() {
        assertTrue(showRetry(LinkPhase.Lost))
        assertTrue(showRetry(LinkPhase.Failed))
        assertFalse(showRetry(LinkPhase.Connected))
        assertFalse(showRetry(LinkPhase.Connecting))
        assertTrue(showRetry(LinkPhase.Off))
    }

    @Test fun words() {
        assertEquals("Controlling Desk", linkText(LinkPhase.Connected, "Desk"))
        assertEquals("Connection to Desk lost. Retry to reconnect.", linkText(LinkPhase.Lost, "Desk"))
        assertEquals("Connecting\u2026", linkText(LinkPhase.Connecting, "Desk"))
    }

    @Test fun start_failures() {
        assertTrue(startFailureText("Desk", "the other device declined").contains("did not allow control"))
        assertTrue(startFailureText("Desk", "timed out").startsWith("Could not reach Desk"))
        assertTrue(startFailureText("Desk", null).startsWith("Could not reach Desk"))
    }

    @Test fun key_press_reports_both_halves() {
        val sent = mutableListOf<Boolean>()
        assertTrue(pressAndRelease { sent += it; true })
        assertEquals(listOf(true, false), sent)
        // A failure of the press stops there and reports it.
        val s2 = mutableListOf<Boolean>()
        assertFalse(pressAndRelease { s2 += it; false })
        assertEquals(listOf(true), s2)
    }

    @Test
    fun an_outdated_relay_says_which_device_to_update() {
        val t = startFailureText("DESK", "reason=the other device runs an older version of Relay: update Relay there")
        org.junit.Assert.assertEquals("DESK runs an older version of Relay. Update Relay on DESK", t)
    }
}
