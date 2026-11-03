package dev.phonebridge.app.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class EntryPointsTest {
    @Test fun every_destination_has_exactly_one_primary_entry_point() {
        for (d in Destination.entries) {
            assertEquals("$d", 1, entryPoints.count { it.dest == d && it.primary })
        }
    }

    @Test fun a_second_entry_point_has_its_own_label_and_a_reason() {
        for (d in Destination.entries) {
            val all = entryPoints.filter { it.dest == d }
            val primary = all.single { it.primary }
            for (extra in all.filter { !it.primary }) {
                assertTrue("${extra.where}: label repeats the primary", extra.label != primary.label)
                assertTrue("${extra.where}: needs a reason", extra.reason.isNotBlank())
            }
        }
    }

    @Test fun no_screen_offers_the_same_destination_twice_under_one_label() {
        val keys = entryPoints.map { Triple(it.where, it.dest, it.label) }
        assertEquals(keys.size, keys.toSet().size)
    }

    @Test fun the_four_tabs_are_primary_in_the_bottom_bar() {
        for (t in Tab.entries) {
            val p = entryPoints.single { it.primary && it.dest.tab == t }
            assertEquals("Bottom bar", p.where)
            assertEquals(t.label, p.label)
        }
    }
}
