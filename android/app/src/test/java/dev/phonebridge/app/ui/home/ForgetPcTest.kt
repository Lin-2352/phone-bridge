package dev.phonebridge.app.ui.home

import dev.phonebridge.app.ui.timeline.isOpenable
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ForgetPcTest {
    @Test
    fun names_the_single_pc() {
        val p = forgetPrompt(listOf("Desk"))
        assertEquals("Remove Desk?", p.title)
        assertTrue(p.body.contains("removes Desk from this phone"))
        assertTrue(p.body.contains("recognise the reset"))
    }

    @Test
    fun names_every_pc() {
        val p = forgetPrompt(listOf("Desk", "Laptop", "Work"))
        assertEquals("Remove these PCs?", p.title)
        assertTrue(p.body.contains("Desk, Laptop and Work"))
    }

    @Test
    fun blank_name_and_duplicates() {
        assertTrue(forgetPrompt(listOf("")).body.contains("removes your PC from"))
        assertTrue(forgetPrompt(listOf("A", "A")).title == "Remove A?")
    }

    @Test
    fun only_saved_content_files_open() {
        assertTrue(isOpenable(true, "content://x/1"))
        assertFalse(isOpenable(true, "file:///x"))
        assertFalse(isOpenable(true, null))
        assertFalse(isOpenable(false, "content://x/1"))
    }
}
