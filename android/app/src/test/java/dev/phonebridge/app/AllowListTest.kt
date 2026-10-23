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

class UniquePeerIdTest {
    private val peers = listOf("My PC" to "id-a", "Office" to "id-b", "My PC" to "id-c")

    @Test
    fun a_name_used_once_gives_its_id() {
        assertEquals("id-b", uniquePeerId(peers, "Office"))
    }

    @Test
    fun a_name_shared_by_two_pcs_gives_no_id_so_it_is_never_auto_allowed() {
        assertEquals("", uniquePeerId(peers, "My PC"))
    }

    @Test
    fun an_unknown_name_gives_no_id() {
        assertEquals("", uniquePeerId(peers, "Nope"))
    }
}

class PruneAllowedTest {
    @Test
    fun a_removed_pc_is_dropped_from_the_list() {
        assertEquals(setOf("a"), pruneAllowed(setOf("a", "b"), listOf("a", "c")))
    }

    @Test
    fun nothing_changes_while_every_listed_pc_is_still_paired() {
        assertEquals(setOf("a", "b"), pruneAllowed(setOf("a", "b"), listOf("a", "b", "c")))
    }
}
