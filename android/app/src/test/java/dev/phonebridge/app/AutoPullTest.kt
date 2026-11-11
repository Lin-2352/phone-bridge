package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class AutoPullTest {
    private val now = 1_000_000L

    private fun verdict(copyTs: Long, lastSent: Long = 0, pcChangedAgo: Long = 0) =
        autoPullVerdict(copyTs, now, lastSent, pcChangedAgo)

    @Test
    fun nothing_copied_sends_nothing() {
        assertEquals(AutoVerdict.NOTHING, verdict(copyTs = 0))
    }

    @Test
    fun a_fresh_copy_is_sent() {
        assertEquals(AutoVerdict.SEND, verdict(copyTs = now - 5_000))
    }

    @Test
    fun a_copy_already_sent_is_not_sent_again() {
        assertEquals(AutoVerdict.UNCHANGED, verdict(copyTs = now - 5_000, lastSent = now - 5_000))
    }

    @Test
    fun a_newer_copy_than_the_last_sent_is_sent() {
        assertEquals(AutoVerdict.SEND, verdict(copyTs = now - 2_000, lastSent = now - 9_000))
    }

    /** The PC copied 3 s ago; the phone's clip is 10 s old: the PC holds the newer one. */
    @Test
    fun a_clip_older_than_the_pcs_own_copy_is_not_sent() {
        assertEquals(AutoVerdict.UNCHANGED, verdict(copyTs = now - 10_000, pcChangedAgo = 3_000))
    }

    @Test
    fun a_clip_newer_than_the_pcs_own_copy_is_sent() {
        assertEquals(AutoVerdict.SEND, verdict(copyTs = now - 1_000, pcChangedAgo = 3_000))
    }

    @Test
    fun an_unknown_pc_copy_age_never_blocks() {
        assertEquals(AutoVerdict.SEND, verdict(copyTs = now - 60_000, pcChangedAgo = 0))
    }

    @Test
    fun a_clip_copied_at_the_same_moment_as_the_pcs_is_the_pcs() {
        assertEquals(AutoVerdict.UNCHANGED, verdict(copyTs = now - 3_000, pcChangedAgo = 3_000))
    }
}
