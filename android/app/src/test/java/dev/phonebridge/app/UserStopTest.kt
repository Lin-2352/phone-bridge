package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class UserStopTest {
    private val byUser = ExitRecord(timestamp = 100, byUserRequest = true)

    @Test fun firstRunOnlyRemembersWhereHistoryStarts() {
        val v = userStopVerdict(0, listOf(byUser))
        assertFalse(v.stopped)
        assertEquals(100L, v.newSeen)
    }

    @Test fun newUserStopIsHonoured() {
        val v = userStopVerdict(50, listOf(byUser))
        assertTrue(v.stopped)
        assertEquals(100L, v.newSeen)
    }

    @Test fun oldStopAlreadySeenIsIgnored() {
        assertFalse(userStopVerdict(100, listOf(byUser)).stopped)
    }

    @Test fun otherExitReasonsAreIgnored() {
        assertFalse(userStopVerdict(50, listOf(ExitRecord(100, false))).stopped)
    }

    @Test fun noHistoryChangesNothing() {
        val v = userStopVerdict(70, emptyList())
        assertFalse(v.stopped)
        assertEquals(70L, v.newSeen)
    }

    @Test fun startingConsumesTheRecordSoItIsNotReadAsNew() {
        // User stopped it (ts 100); later the user turns it on again: that stop is now history.
        val consumed = userStopVerdict(50, listOf(byUser)).newSeen
        assertFalse(userStopVerdict(consumed, listOf(byUser)).stopped)
    }
}
