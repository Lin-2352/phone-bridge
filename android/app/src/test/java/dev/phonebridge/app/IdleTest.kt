package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class IdleTest {
    @Test
    fun no_sampling_when_idle() {
        // No PC session and no Relay screen: no timer at all, screen on or off.
        assertNull(sampleIntervalMs(sessionActive = false, uiVisible = false, interactive = true))
        assertNull(sampleIntervalMs(sessionActive = false, uiVisible = false, interactive = false))
    }

    @Test
    fun sampling_during_a_session() {
        assertEquals(10_000L, sampleIntervalMs(sessionActive = true, uiVisible = false, interactive = true))
        assertEquals(60_000L, sampleIntervalMs(sessionActive = true, uiVisible = false, interactive = false))
        assertEquals(10_000L, sampleIntervalMs(sessionActive = false, uiVisible = true, interactive = true))
    }

    @Test
    fun status_redraws_only_while_someone_can_see_it() {
        assertTrue(statusRedrawDue(screenOn = true, ready = true, connected = false))
        assertFalse(statusRedrawDue(screenOn = false, ready = true, connected = false)) // no wakeups with the screen off
        assertFalse(statusRedrawDue(screenOn = true, ready = true, connected = true)) // "Connected" doesn't age
        assertFalse(statusRedrawDue(screenOn = true, ready = false, connected = false))
    }
}
