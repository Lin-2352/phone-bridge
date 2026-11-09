package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class StatusLineTest {
    private fun st(
        running: Boolean = true, ready: Boolean = true, pc: String? = "DESK", connected: Boolean = false,
        controller: String? = null, failed: Boolean = false,
    ) = relayStatus(running, ready, pc, connected, controller, attemptFailed = failed)

    @Test fun a_taken_discovery_port_is_said_when_the_pc_is_away() {
        val s = relayStatus(true, true, "DESK", false, null, discoveryOff = true)
        assertEquals("DESK not reachable", s.title)
        assertEquals("Can't look for it on a new network: another app uses port 47199", s.detail)
        // Connected: nothing to say.
        assertNull(relayStatus(true, true, "DESK", true, null, discoveryOff = true).detail)
    }

    @Test fun off_starting_unpaired() {
        assertEquals("$APP_NAME is off", st(running = false).title)
        assertEquals("Starting\u2026", st(ready = false).title)
        assertEquals("Not paired yet", st(pc = null).title)
    }

    @Test fun connected_only_with_a_live_session() {
        assertEquals("Connected to DESK", st(connected = true).title)
        assertNull(st(connected = true).detail)
    }

    // Relay connects only when there is something to do, so no open session says nothing about the PC.
    @Test fun no_open_session_is_not_called_unreachable() {
        val s = st()
        assertEquals("Paired with DESK", s.title)
        assertNull(s.detail)
        assertEquals("Paired with DESK", s.line)
    }

    @Test fun only_a_failed_attempt_says_not_reachable() {
        val s = st(failed = true)
        assertEquals("DESK not reachable", s.title)
        assertNull(s.detail)
    }

    @Test fun a_live_session_outranks_an_old_failure() {
        assertEquals("Connected to DESK", st(connected = true, failed = true).title)
    }

    @Test fun controlling_wins() {
        assertEquals("DESK is controlling this phone", st(controller = "DESK", connected = true).title)
    }

    @Test fun the_status_line_never_carries_a_last_seen_time() {
        // Last seen lives on the Home card only (owner, 2026-11-09).
        assertNull(st().detail)
        assertNull(st(failed = true).detail)
    }

    /** M3.7 (B5): the service has stopped on purpose, so "Relay is off" would send the user the wrong way. */
    @Test fun a_lost_identity_is_said_before_anything_else() {
        val s = relayStatus(false, false, "DESK", false, identityLost = true)
        assertEquals("Relay can't open its keys", s.title)
    }
}
