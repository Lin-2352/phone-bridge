package dev.phonebridge.app.ui.timeline

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DayExpansionTest {
    @Test fun up_to_three_days_all_start_expanded() {
        assertTrue(isDayExpanded("Yesterday", isToday = false, groupCount = 3, overrides = emptyMap()))
    }

    @Test fun more_than_three_days_only_today_starts_expanded() {
        assertTrue(isDayExpanded("Today", true, 4, emptyMap()))
        assertFalse(isDayExpanded("Yesterday", false, 4, emptyMap()))
    }

    @Test fun a_user_choice_beats_the_default() {
        assertTrue(isDayExpanded("Yesterday", false, 4, mapOf("Yesterday" to true)))
        assertFalse(isDayExpanded("Today", true, 4, mapOf("Today" to false)))
    }

    @Test fun toggle_flips_the_current_state() {
        val o = toggleDay(emptyMap(), "Yesterday", currentlyExpanded = false)
        assertEquals(mapOf("Yesterday" to true), o)
        assertEquals(mapOf("Yesterday" to false), toggleDay(o, "Yesterday", currentlyExpanded = true))
    }

    @Test fun collapse_and_expand_all() {
        assertEquals(mapOf("a" to false, "b" to false), setAllDays(listOf("a", "b"), expanded = false))
        assertEquals(mapOf("a" to true, "b" to true), setAllDays(listOf("a", "b"), expanded = true))
    }

    @Test fun all_expanded_reads_every_day() {
        val days = listOf("Today" to true, "Yesterday" to false, "Mon 3 Nov" to false, "Sun 2 Nov" to false)
        assertFalse(allDaysExpanded(days, emptyMap()))
        assertTrue(allDaysExpanded(days, setAllDays(days.map { it.first }, true)))
    }

    @Test fun count_text() {
        assertEquals("1 item", itemCountText(1))
        assertEquals("12 items", itemCountText(12))
    }
}
