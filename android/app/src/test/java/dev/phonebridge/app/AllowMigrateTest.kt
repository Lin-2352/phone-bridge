package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class AllowMigrateTest {
    @Test fun oldGlobalOnBecomesTheAlreadyPairedPcs() {
        assertEquals(setOf("a"), migrateGlobalAllow(oldOn = true, current = emptySet(), pairedIds = listOf("a")))
        assertEquals(setOf("a", "b"), migrateGlobalAllow(true, setOf("a"), listOf("a", "b")))
    }

    @Test fun oldGlobalOffChangesNothing() {
        assertEquals(setOf("x"), migrateGlobalAllow(false, setOf("x"), listOf("a")))
    }

    @Test fun noPairedPcLeavesTheSetAlone() {
        assertEquals(emptySet<String>(), migrateGlobalAllow(true, emptySet(), emptyList()))
    }

    @Test fun emptyIdsAreNeverAllowed() {
        assertEquals(setOf("a"), migrateGlobalAllow(true, emptySet(), listOf("", "a")))
    }
}
