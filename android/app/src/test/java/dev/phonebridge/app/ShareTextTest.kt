package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ShareTextTest {
    /** Stands in for a Spanned: a CharSequence that is not a String (Bundle.getString would drop it). */
    private class Styled(private val s: String) : CharSequence by s {
        override fun toString() = s
    }

    @Test fun styledTextIsKept() {
        assertEquals("hello world", sharedText(Styled("hello world")))
    }

    @Test fun plainTextIsKept() = assertEquals("x", sharedText("x"))

    @Test fun blankOrMissingIsNothing() {
        assertNull(sharedText(null))
        assertNull(sharedText("  \n"))
        assertNull(sharedText(Styled("")))
    }

    @Test fun toastOnlyPromisesWhatIsSent() {
        assertEquals("Sending to DESK", shareToast("DESK", files = 1, hasText = false, dropped = 0))
        assertEquals("Sending to DESK", shareToast("DESK", files = 0, hasText = true, dropped = 0))
        assertEquals("Sending to your PC", shareToast(null, files = 0, hasText = true, dropped = 0))
        assertEquals("Nothing to send", shareToast("DESK", files = 0, hasText = false, dropped = 0))
        assertEquals("Sending 2 files to DESK. 1 file could not be read", shareToast("DESK", files = 2, hasText = false, dropped = 1))
        assertEquals("Sending to DESK. 1 file could not be read", shareToast("DESK", files = 1, hasText = false, dropped = 1))
    }
}
