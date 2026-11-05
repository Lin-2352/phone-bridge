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

class AskPeerIdTest {
    private val paired = listOf("id-a", "id-b")

    @Test
    fun a_paired_id_is_kept() {
        assertEquals("id-b", askPeerId("id-b", paired))
    }

    @Test
    fun an_unpaired_or_empty_id_gives_none() {
        assertEquals("", askPeerId("id-z", paired))
        assertEquals("", askPeerId("", paired))
    }

    @Test
    fun same_name_other_id_is_not_allowed() {
        // Two PCs both called "DESK": only the one in the list starts without asking.
        val allowed = setOf("id-a")
        assertEquals(true, allowedFor(askPeerId("id-a", paired), allowed))
        assertEquals(false, allowedFor(askPeerId("id-b", paired), allowed))
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
