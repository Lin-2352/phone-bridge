package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AllowListTest {
    @Test
    fun an_unlisted_pc_is_not_allowed() {
        assertFalse(allowedFor("pc-b", setOf("pc-a")))
    }

    @Test
    fun a_listed_pc_is_allowed() {
        assertTrue(allowedFor("pc-a", setOf("pc-a")))
    }

    @Test
    fun allowing_adds_only_that_pc() {
        assertEquals(setOf("pc-a", "pc-b"), withAllowed(setOf("pc-a"), "pc-b", true))
    }

    @Test
    fun revoking_removes_only_that_pc() {
        assertEquals(setOf("pc-a"), withAllowed(setOf("pc-a", "pc-b"), "pc-b", false))
    }

    @Test
    fun an_empty_peer_id_is_never_allowed() {
        assertFalse(allowedFor("", setOf("")))
        assertEquals(setOf("pc-a"), withAllowed(setOf("pc-a"), "", true))
    }
}
