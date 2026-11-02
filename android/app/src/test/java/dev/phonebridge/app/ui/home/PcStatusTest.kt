package dev.phonebridge.app.ui.home

import org.junit.Assert.assertEquals
import org.junit.Test

class PcStatusTest {
    private val now = 1_800_000_000_000L
    private val min = 60_000L

    @Test
    fun connected_only_while_something_is_live() {
        assertEquals("Connected", pcStatus(live = true, lastSeenMs = now - 90 * min, nowMs = now, paired = true))
        assertEquals("Last seen 5 min ago", pcStatus(live = false, lastSeenMs = now - 5 * min, nowMs = now, paired = true))
    }

    @Test
    fun never_seen_says_so() {
        assertEquals("Not seen yet", pcStatus(live = false, lastSeenMs = 0L, nowMs = now, paired = false))
    }

    @Test
    fun paired_without_a_stamp_says_paired() {
        // Upgraded from a version that kept no "last seen": the PC has reached this phone before.
        assertEquals("Paired", pcStatus(live = false, lastSeenMs = 0L, nowMs = now, paired = true))
        assertEquals("Connected", pcStatus(live = true, lastSeenMs = 0L, nowMs = now, paired = true))
    }

    @Test
    fun ago_matches_the_pc_wording() {
        assertEquals("Last seen just now", pcStatus(false, now - 59_000, now, true))
        assertEquals("Last seen 1 min ago", pcStatus(false, now - 60_000, now, true))
        assertEquals("Last seen 2 h ago", pcStatus(false, now - 125 * min, now, true))
        assertEquals("Last seen yesterday", pcStatus(false, now - 30 * 60 * min, now, true))
        assertEquals("Last seen 3 days ago", pcStatus(false, now - 3 * 24 * 60 * min, now, true))
        // A clock that moved backwards is not "in the future".
        assertEquals("Last seen just now", pcStatus(false, now + 5 * min, now, true))
    }
}
