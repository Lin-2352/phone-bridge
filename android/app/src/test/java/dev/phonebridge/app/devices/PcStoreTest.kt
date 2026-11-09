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
    fun the_devices_list_never_shows_a_last_seen_time() {
        // Last seen lives on Home only (owner, 2026-11-09): here a PC is connected or it is not.
        assertEquals("Connected", pcStatus(live = true, paired = true))
        assertEquals("Not connected", pcStatus(live = false, paired = true))
        assertEquals("Not paired", pcStatus(live = false, paired = false))
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

    private val idA = "a".repeat(64)
    private val idB = "b".repeat(64)

    @Test
    fun re_pairing_a_removed_device_starts_at_fresh_defaults() {
        val keys = setOf("$idA/never_screen", "$idA/auto_accept", "$idB/clipboard_sync", "default_pc", "theme", "allow_audio_pcs")
        assertEquals(listOf("$idA/never_screen", "$idA/auto_accept"), keysOfUnpaired(keys, listOf(idB)))
        assertEquals("other keys are never touched", emptyList<String>(), keysOfUnpaired(keys - setOf("$idA/never_screen", "$idA/auto_accept"), listOf(idB)))
    }

    @Test
    fun the_default_moves_on_when_its_pc_is_gone() {
        assertEquals(idB, defaultIfStillPaired(idA, listOf(idB)))
        assertEquals(idB, defaultIfStillPaired(idB, listOf(idA, idB)))
        assertNull(defaultIfStillPaired(idA, emptyList()))
    }

    @Test
    fun one_pc_needs_no_picker() {
        assertEquals(SendTarget.One("a"), sendTarget(listOf(peer("a", "Alpha")), null, null))
    }

    @Test
    fun several_pcs_ask_with_the_default_first() {
        val ps = listOf(peer("b", "bravo"), peer("a", "Alpha"))
        assertEquals(SendTarget.Ask(listOf("b", "a")), sendTarget(ps, null, "b"))
        assertEquals("no default: by name", SendTarget.Ask(listOf("a", "b")), sendTarget(ps, null, null))
    }

    @Test
    fun an_explicit_pc_skips_the_picker() {
        val ps = listOf(peer("b", "bravo"), peer("a", "Alpha"))
        assertEquals(SendTarget.One("a"), sendTarget(ps, "a", "b"))
    }

    @Test
    fun an_unpaired_explicit_pc_falls_back_to_asking() {
        val ps = listOf(peer("b", "bravo"), peer("a", "Alpha"))
        assertEquals(SendTarget.Ask(listOf("a", "b")), sendTarget(ps, "gone", null))
    }

    @Test
    fun no_pc_means_pair_first() {
        assertEquals(SendTarget.None, sendTarget(emptyList(), "x", "x"))
    }

    @Test
    fun tile_click_targets_the_default() {
        assertEquals("To Desk", tileSubtitle("Desk"))
        assertEquals("To your PC", tileSubtitle(null))
        assertEquals("To your PC", tileSubtitle("  "))
    }

    @Test
    fun widget_label_names_the_default() {
        assertEquals("Desk", widgetLabel("Desk", "Connected"))
        assertEquals("Not paired", widgetLabel(null, "Not paired"))
    }

    @Test
    fun no_default_with_several_pcs_opens_the_picker() {
        val ps = listOf(peer("a", "Alpha"), peer("b", "Bravo"))
        assertEquals(true, needsPicker(ps, null))
        assertEquals(true, needsPicker(ps, "gone"))
        assertEquals(false, needsPicker(ps, "b"))
        assertEquals(false, needsPicker(listOf(peer("a", "Alpha")), null))
    }

    @Test
    fun the_all_pcs_switch_reads_on_only_when_every_pc_has_it_on() {
        // A freshly paired PC starts off, so the master switch must not claim "on" for it.
        assertEquals(false, allPcsSwitch(listOf(true, false), stored = true))
        assertEquals(true, allPcsSwitch(listOf(true, true), stored = false))
        assertEquals(false, allPcsSwitch(listOf(false, false), stored = true))
        // No PC list yet (Relay is off): the stored value stands.
        assertEquals(true, allPcsSwitch(emptyList(), stored = true))
        assertEquals(false, allPcsSwitch(emptyList(), stored = false))
    }
}
