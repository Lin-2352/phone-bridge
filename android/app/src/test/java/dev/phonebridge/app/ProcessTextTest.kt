package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ProcessTextTest {
    @Test
    fun selected_text_is_sent_as_it_is() {
        assertEquals("hello world", selectedTextToSend("hello world"))
    }

    @Test
    fun nothing_or_blank_selected_sends_nothing() {
        assertNull(selectedTextToSend(null))
        assertNull(selectedTextToSend(""))
        assertNull(selectedTextToSend("   \n"))
    }

    @Test
    fun very_long_text_is_capped() {
        val sent: String = selectedTextToSend("a".repeat(PROCESS_TEXT_MAX + 500)) ?: error("expected text")
        assertEquals(PROCESS_TEXT_MAX, sent.length)
    }
}
