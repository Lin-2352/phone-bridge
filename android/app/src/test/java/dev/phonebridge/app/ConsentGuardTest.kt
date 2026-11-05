package dev.phonebridge.app

import android.view.MotionEvent
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

class ConsentGuardTest {
    @Before fun clear() = ConsentGuard.clearForTest()

    @Test fun guardIsActiveUntilEveryPromptEnds() {
        ConsentGuard.begin("control", 0)
        ConsentGuard.begin("capture", 1)
        assertTrue(ConsentGuard.active(2))
        ConsentGuard.end("control")
        assertTrue("a second prompt is still up", ConsentGuard.active(3))
        ConsentGuard.end("capture")
        assertFalse(ConsentGuard.active(4))
    }

    /** A prompt killed without reporting back must not pause remote input forever. */
    @Test fun guardExpiresAfter120s() {
        ConsentGuard.begin("control", 0)
        assertTrue(ConsentGuard.active(119_999))
        assertFalse(ConsentGuard.active(120_000))
    }

    @Test fun obscuredFlagsAreDetected() {
        assertTrue(isObscured(MotionEvent.FLAG_WINDOW_IS_OBSCURED))
        assertTrue(isObscured(MotionEvent.FLAG_WINDOW_IS_PARTIALLY_OBSCURED))
        assertFalse(isObscured(0))
    }
}
