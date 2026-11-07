package dev.phonebridge.app.devices

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import uniffi.pb_ffi.PbPeer

class PcStoreTest {
    private fun peer(id: String, name: String, fp: String = "ABCD1234") = PbPeer(id, name, fp)

    @Test
    fun nickname_wins_over_name() {
        assertEquals("Desk PC", displayName("Desk PC", "ASUS-ROG"))
    }

    @Test
    fun blank_nickname_falls_back() {
        assertEquals("ASUS-ROG", displayName(null, "ASUS-ROG"))
        assertEquals("ASUS-ROG", displayName("   ", "ASUS-ROG"))
        assertEquals("control characters are dropped", "ASUS-ROG", displayName("\u0000\u0007", "ASUS-ROG"))
    }

    @Test
    fun a_nickname_is_capped_at_64_characters() {
        assertEquals(64, displayName("x".repeat(200), "n").length)
    }

    @Test
    fun duplicate_names_get_a_fingerprint_suffix() {
        val l = labels(listOf(peer("1", "PC", "1A2B3C"), peer("2", "PC", "9Z8Y7X"), peer("3", "Other")), emptyMap())
        assertEquals("PC (1A2B)", l["1"])
        assertEquals("PC (9Z8Y)", l["2"])
        assertEquals("Other", l["3"])
    }

    @Test
    fun a_nickname_resolves_a_duplicate_without_a_suffix() {
        val l = labels(listOf(peer("1", "PC", "1A2B3C"), peer("2", "PC", "9Z8Y7X")), mapOf("1" to "Desk"))
        assertEquals("Desk", l["1"])
        assertEquals("the other one is no longer a duplicate", "PC", l["2"])
    }

    @Test
    fun removing_the_default_picks_the_next_by_label() {
        val remaining = listOf(peer("b", "bravo"), peer("a", "Alpha"))
        assertEquals("a", defaultAfterRemoval(remaining, removed = "x", current = "x"))
    }

    @Test
    fun removing_another_keeps_the_default() {
        val remaining = listOf(peer("a", "Alpha"), peer("b", "Bravo"))
        assertEquals("b", defaultAfterRemoval(remaining, removed = "z", current = "b"))
    }

    @Test
    fun removing_the_last_device_leaves_no_default() {
        assertNull(defaultAfterRemoval(emptyList(), removed = "a", current = "a"))
    }

    @Test
    fun status_per_pc_uses_its_own_last_seen() {
        val now = 10_000_000L
        assertEquals("Connected", pcStatus(live = true, lastSeenMs = 0, nowMs = now, paired = true))
        assertEquals("Last seen 5 min ago", pcStatus(live = false, lastSeenMs = now - 5 * 60_000, nowMs = now, paired = true))
        assertEquals("Not seen yet", pcStatus(live = false, lastSeenMs = 0, nowMs = now, paired = true))
        assertEquals("Not paired", pcStatus(live = false, lastSeenMs = now, nowMs = now, paired = false))
    }

    @Test
    fun a_new_pc_starts_at_ask() {
        val d = freshDefaults()
        assertEquals(listOf(Allow.Ask, Allow.Ask, Allow.Ask, Allow.Ask), listOf(d.control, d.screen, d.camera, d.audio))
        assertEquals(listOf(false, false, false), listOf(d.clipboardSync, d.notifications, d.autoAccept))
        // A PC with no stored keys reads as the fresh defaults.
        assertEquals(d, pcSettings(emptyMap(), "pc1"))
    }

    @Test
    fun unknown_values_read_as_ask() {
        val all = mapOf<String, Any?>("pc1/control" to "sometimes", "pc1/screen" to "always", "pc1/camera" to "never")
        val p = pcSettings(all, "pc1")
        assertEquals(Allow.Ask, p.control)
        assertEquals(Allow.Always, p.screen)
        assertEquals(Allow.Never, p.camera)
    }
}
