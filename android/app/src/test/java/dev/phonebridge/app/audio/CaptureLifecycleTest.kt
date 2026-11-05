package dev.phonebridge.app.audio

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CaptureLifecycleTest {
    @Test
    fun first_teardown_wins() {
        val c = CaptureLifecycle()
        assertTrue(c.beginTeardown())
        assertFalse(c.beginTeardown())
    }

    @Test
    fun source_end_after_teardown_is_ignored() {
        // A Stop is already tearing down: the capture thread must not stop again (or stop the next share).
        val c = CaptureLifecycle()
        c.beginTeardown()
        assertFalse(c.onSourceEnded())
    }

    @Test
    fun teardown_after_source_end_is_ignored() {
        val c = CaptureLifecycle()
        assertTrue(c.onSourceEnded())
        assertFalse(c.beginTeardown())
        assertFalse(c.onSourceEnded())
    }
}
