package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LiveStoreTest {
    @Test
    fun oldest_entry_is_dropped_past_the_limit() {
        val s = LiveStore<String>(max = 2)
        s.put("a", "A")
        s.put("b", "B")
        s.get("a") // used recently: kept
        s.put("c", "C")
        assertEquals("A", s.get("a"))
        assertNull(s.get("b"))
        assertEquals("C", s.get("c"))
    }

    @Test
    fun put_says_whether_it_replaced() {
        val s = LiveStore<String>()
        assertFalse(s.put("a", "A"))
        assertTrue(s.put("a", "A2"))
        assertEquals("A2", s.remove("a"))
        assertNull(s.remove("a"))
    }

    @Test
    fun clear_empties_the_store() {
        val s = LiveStore<String>()
        s.put("a", "A")
        s.clear()
        assertNull(s.get("a"))
    }

    @Test
    fun call_category_is_found() {
        val s = LiveStore<LiveNote<String, String>>()
        s.put("x", LiveNote(category = "msg", title = null, content = null, actions = emptyList()))
        assertFalse(s.any { it.category == "call" })
        s.put("y", LiveNote(category = "call", title = "Ana", content = "open", actions = listOf(LiveAction("answer", null))))
        assertTrue(s.any { it.category == "call" })
    }
}
