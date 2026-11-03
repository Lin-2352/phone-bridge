package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TransferTextTest {
    @Test fun cancelReasonsAreRecognised() {
        assertTrue(isCancelled("transfer cancelled"))
        assertTrue(isCancelled("the other device cancelled the transfer"))
        assertFalse(isCancelled("connection lost"))
    }

    @Test fun progressLineHasSizesSpeedAndEta() {
        assertEquals("46 MB of 400 MB \u00b7 11 MB/s \u00b7 30 s left", transferLine(46_000_000, 400_000_000, 4_000))
    }

    @Test fun progressLineWithoutSpeedYet() {
        assertEquals("0 B of 10 MB", transferLine(0, 10_000_000, 100))
    }

    @Test fun failureWordingOffersRetryOnlyWhenPossible() {
        assertEquals("Sending cancelled", failureTitle("a.zip", cancelled = true, outgoing = true))
        assertEquals("Could not send a.zip", failureTitle("a.zip", cancelled = false, outgoing = true))
        assertEquals("Receiving a.zip cancelled", failureTitle("a.zip", cancelled = true, outgoing = false))
    }

    @Test fun retryStoreKeepsTheNewestAndReturnsEvicted() {
        val s = RetryStore<String>(2)
        assertTrue(s.put(1, "a").isEmpty())
        assertTrue(s.put(2, "b").isEmpty())
        assertEquals(listOf("a"), s.put(3, "c"))
        assertNull(s.take(1))
        assertEquals("b", s.take(2))
        assertNull(s.take(2))
    }
}
