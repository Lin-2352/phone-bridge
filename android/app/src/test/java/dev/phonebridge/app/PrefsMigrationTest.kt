package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertTrue
import org.junit.Test

class PrefsMigrationTest {
    private val old = mapOf<String, Any?>(
        "last_clip" to 123,
        "allow_camera" to true,
        "allow_control" to true, // pending global switch: folded at each start, not by a version step
        "allow_control_pcs" to setOf("a"),
        "theme" to 2,
    )

    @Test
    fun version_is_zero_for_old_prefs_and_current_for_a_fresh_install() {
        assertEquals(0, prefsVersion(old))
        assertEquals(PREFS_VERSION, prefsVersion(emptyMap()))
        assertEquals(1, prefsVersion(mapOf(PREFS_VERSION_KEY to 1)))
    }

    @Test
    fun each_legacy_key_is_handled() {
        val (v, out) = migratePrefs(0, old)
        assertEquals(PREFS_VERSION, v)
        assertTrue("last_clip" !in out && "allow_camera" !in out)
        assertEquals(true, out["allow_control"]) // still pending
        assertEquals(setOf("a"), out["allow_control_pcs"])
        assertEquals(2, out["theme"])
    }

    @Test
    fun current_version_changes_nothing() {
        val now = old + (PREFS_VERSION_KEY to PREFS_VERSION)
        val (v, out) = migratePrefs(PREFS_VERSION, now)
        assertEquals(PREFS_VERSION, v)
        assertEquals(now, out)
    }

    @Test
    fun future_version_is_left_alone() {
        val future = old + (PREFS_VERSION_KEY to PREFS_VERSION + 5)
        val (v, out) = migratePrefs(PREFS_VERSION + 5, future)
        assertEquals(PREFS_VERSION + 5, v)
        assertEquals(future, out)
    }

    @Test
    fun string_sets_are_copied() {
        val (_, out) = migratePrefs(0, old)
        assertNotSame(old["allow_control_pcs"], out["allow_control_pcs"])
    }

    @Test
    fun the_diff_lists_only_what_changed() {
        val (v, out) = migratePrefs(0, old)
        val d = prefsDiff(old, out + (PREFS_VERSION_KEY to v))
        assertEquals(setOf("last_clip", "allow_camera"), d.remove)
        assertEquals(mapOf<String, Any>(PREFS_VERSION_KEY to PREFS_VERSION), d.put)
    }
}
