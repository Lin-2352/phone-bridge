package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class StatusLineTest {
    private val now = 10_000_000_000L
    private fun st(
        running: Boolean = true, ready: Boolean = true, pc: String? = "DESK", connected: Boolean = false,
        seen: Long = 0L, controller: String? = null, failed: Boolean = false,
    ) = relayStatus(running, ready, pc, connected, seen, now, controller, attemptFailed = failed)

    @Test fun a_taken_discovery_port_is_said_when_the_pc_is_away() {
        val s = relayStatus(true, true, "DESK", false, now - 3 * 3600_000L, now, null, discoveryOff = true)
        assertEquals("DESK not reachable", s.title)
        assertEquals("Can't look for it on a new network: another app uses port 47199", s.detail)
        // Connected: nothing to say.
        assertNull(relayStatus(true, true, "DESK", true, now, now, null, discoveryOff = true).detail)
    }

    @Test fun off_starting_unpaired() {
        assertEquals("$APP_NAME is off", st(running = false).title)
        assertEquals("Starting\u2026", st(ready = false).title)
        assertEquals("Not paired yet", st(pc = null).title)
    }

    @Test fun connected_only_with_a_live_session() {
        assertEquals("Connected to DESK", st(connected = true, seen = now).title)
        assertNull(st(connected = true).detail)
    }

    // Relay connects only when there is something to do, so no open session says nothing about the PC.
    @Test fun no_open_session_is_not_called_unreachable() {
        val s = st(seen = now - 3 * 3600_000L)
        assertEquals("Paired with DESK", s.title)
        assertEquals("Last seen 3 h ago", s.detail)
        assertEquals("Paired with DESK \u00b7 last seen 3 h ago", s.line)
    }

    @Test fun only_a_failed_attempt_says_not_reachable() {
        val s = st(seen = now - 3 * 3600_000L, failed = true)
        assertEquals("DESK not reachable", s.title)
        assertEquals("Last seen 3 h ago", s.detail)
        val never = st(failed = true)
        assertEquals("DESK not reachable", never.title)
        assertNull(never.detail)
    }

    @Test fun a_live_session_outranks_an_old_failure() {
        assertEquals("Connected to DESK", st(connected = true, seen = now, failed = true).title)
    }

    @Test fun paired_without_any_contact_stamp_is_only_the_paired_fact() {
        val s = st(seen = 0L)
        assertEquals("Paired with DESK", s.title)
        assertNull(s.detail)
    }

    @Test fun controlling_wins() {
        assertEquals("DESK is controlling this phone", st(controller = "DESK", connected = true).title)
    }

    @Test fun staleness_is_the_same_wording_as_home() {
        assertEquals("Last seen just now", st(seen = now - 5_000).detail)
        assertEquals("Last seen yesterday", st(seen = now - 30 * 3600_000L).detail)
    }

    /** M3.7 (B5): the service has stopped on purpose, so "Relay is off" would send the user the wrong way. */
    @Test fun a_lost_identity_is_said_before_anything_else() {
        val s = relayStatus(false, false, "DESK", false, 0L, now, identityLost = true)
        assertEquals("Relay can't open its keys", s.title)
    }
}
