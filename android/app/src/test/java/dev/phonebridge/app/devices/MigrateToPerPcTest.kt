package dev.phonebridge.app.devices

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

class MigrateToPerPcTest {
    private val old: Map<String, Any?> = mapOf(
        "clipboard_sync" to true,
        "notify_to_pc" to true,
        "auto_accept" to true,
        "audio_skip_confirm_phone_to_pc" to true,
        "allow_control_pcs" to setOf("a"),
        "last_seen_ms" to 123L,
    )

    @Test
    fun old_single_pc_install_migrates_to_its_pc() {
        val m = migrateToPerPc(old, listOf("a"))
        assertEquals(true, m["a/clipboard_sync"])
        assertEquals(true, m["a/notifications"])
        assertEquals(true, m["a/auto_accept"])
        assertEquals(setOf("a"), m["allow_audio_pcs"])
        assertEquals("a", m["default_pc"])
        assertEquals(123L, m["a/last_seen_ms"])
        assertEquals("the old keys stay for a downgrade", true, m["clipboard_sync"])
    }

    @Test
    fun allow_lists_stay_always() {
        val m = migrateToPerPc(old, listOf("a", "b"))
        assertEquals(setOf("a"), m["allow_control_pcs"])
    }

    @Test
    fun migration_runs_once() {
        val once = migrateToPerPc(old, listOf("a"))
        val changedByUser = once + ("a/auto_accept" to false)
        assertEquals(changedByUser, migrateToPerPc(changedByUser, listOf("a")))
    }

    @Test
    fun a_pc_paired_later_gets_fresh_defaults() {
        val once = migrateToPerPc(old, listOf("a"))
        val later = migrateToPerPc(once, listOf("a", "b"))
        assertNull(later["b/auto_accept"])
        assertNull(later["b/clipboard_sync"])
        assertFalse(later["allow_audio_pcs"].toString().contains("b"))
        val s = pcSettings(later, "b")
        assertEquals(freshDefaults(), s)
    }

    @Test
    fun two_paired_pcs_both_keep_todays_behaviour() {
        val m = migrateToPerPc(old, listOf("a", "b"))
        for (id in listOf("a", "b")) {
            assertEquals(true, m["$id/clipboard_sync"])
            assertEquals(true, m["$id/auto_accept"])
        }
        assertEquals(setOf("a", "b"), m["allow_audio_pcs"])
    }

    @Test
    fun defaults_that_were_on_stay_on_and_choices_stay() {
        val untouched = migrateToPerPc(mapOf("clipboard_sync" to false), listOf("a"))
        assertEquals("never chosen: both were on", true, untouched["a/auto_accept"])
        assertEquals(true, untouched["a/notifications"])
        assertNull("clipboard sync was off", untouched["a/clipboard_sync"])
        val off = migrateToPerPc(mapOf("auto_accept" to false, "notify_to_pc" to false), listOf("a"))
        assertNull(off["a/auto_accept"])
        assertNull(off["a/notifications"])
    }

    @Test
    fun nothing_paired_is_marked_done_so_a_later_pc_starts_fresh() {
        val m = migrateToPerPc(old, emptyList())
        assertEquals(true, m["pcs_migrated"])
        assertNull(m["a/auto_accept"])
        assertEquals(m, migrateToPerPc(m, listOf("a")))
    }
}
