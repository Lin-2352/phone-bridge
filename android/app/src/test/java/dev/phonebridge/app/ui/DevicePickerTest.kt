package dev.phonebridge.app.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.pb_ffi.PbPeer

class DevicePickerTest {
    private fun peer(id: String, name: String, fp: String = "ABCD1234") = PbPeer(id, name, fp)

    @Test
    fun default_comes_first() {
        val rows = pickerRows(
            peers = listOf(peer("a", "Alpha"), peer("b", "Bravo")),
            live = setOf("a", "b"),
            lastSeen = emptyMap(),
            defaultId = "b",
            nowMs = 0,
        )
        assertEquals("b", rows.first().id)
        assertTrue(rows.first().isDefault)
    }

    @Test
    fun live_before_offline() {
        val rows = pickerRows(
            peers = listOf(peer("a", "Alpha"), peer("b", "Bravo")),
            live = setOf("b"),
            lastSeen = emptyMap(),
            defaultId = null,
            nowMs = 0,
        )
        assertEquals(listOf("b", "a"), rows.map { it.id })
    }

    @Test
    fun ties_sort_by_name_ignoring_case() {
        val rows = pickerRows(
            peers = listOf(peer("1", "bravo"), peer("2", "Alpha"), peer("3", "charlie")),
            live = emptySet(),
            lastSeen = emptyMap(),
            defaultId = null,
            nowMs = 0,
        )
        assertEquals(listOf("Alpha", "bravo", "charlie"), rows.map { it.label })
    }

    @Test
    fun duplicate_names_get_a_fingerprint_suffix() {
        val rows = pickerRows(
            peers = listOf(peer("1", "Pixel", "1A2B3C4D"), peer("2", "Pixel", "9Z8Y7X6W")),
            live = emptySet(),
            lastSeen = emptyMap(),
            defaultId = null,
            nowMs = 0,
        )
        assertEquals(setOf("Pixel (1A2B)", "Pixel (9Z8Y)"), rows.map { it.label }.toSet())
    }
}
