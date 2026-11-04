package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Test

class SilencerPlanTest {
    /** A share left at step 0 must not make the next one silent on the PC for no visible reason. */
    @Test
    fun a_share_starts_at_the_level_of_the_last_one() {
        assertEquals(11, shareStartIndex(remembered = 11, max = 16))
    }

    @Test
    fun the_first_share_starts_halfway() {
        assertEquals(8, shareStartIndex(remembered = -1, max = 16))
    }

    @Test
    fun a_last_share_left_silent_starts_halfway() {
        assertEquals(8, shareStartIndex(remembered = 0, max = 16))
    }

    @Test
    fun a_remembered_level_never_exceeds_this_phones_range() {
        assertEquals(15, shareStartIndex(remembered = 25, max = 15))
    }

    @Test
    fun the_phone_gets_its_own_volume_back() {
        assertEquals(-15, stepsBack(current = 16, saved = 1))
        assertEquals(3, stepsBack(current = 1, saved = 4))
        assertEquals(0, stepsBack(current = 5, saved = 5))
    }

    /** Nothing saved means no share was running: the user's volume is not Relay's to move. */
    @Test
    fun with_nothing_saved_the_volume_stays_where_it_is() {
        assertEquals(0, stepsBack(current = 16, saved = -1))
    }
}
