package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class StatusLineTest {
    private val now = 10_000_000_000L
    private fun st(
        running: Boolean = true, ready: Boolean = true, pc: String? = "DESK", connected: Boolean = false,
        seen: Long = 0L, controller: String? = null,
    ) = relayStatus(running, ready, pc, connected, seen, now, controller)

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

    @Test fun a_paired_pc_not_in_contact_is_not_called_healthy() {
        val s = st(seen = now - 3 * 3600_000L)
        assertEquals("DESK not reachable", s.title)
        assertEquals("Last seen 3 h ago", s.detail)
        assertEquals("DESK not reachable \u00b7 last seen 3 h ago", s.line)
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
}
