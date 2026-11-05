package dev.phonebridge.app.widget

import dev.phonebridge.app.APP_NAME
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class WidgetStateTest {
    @Test
    fun labels_cover_off_unpaired_connected_unreachable() {
        assertEquals("$APP_NAME is off", widgetLabel(running = false, pcName = "DESK", connected = true))
        assertEquals("$APP_NAME is off", widgetLabel(running = false, pcName = null, connected = false))
        assertEquals("Not paired yet", widgetLabel(running = true, pcName = null, connected = false))
        assertEquals("Connected to DESK", widgetLabel(running = true, pcName = "DESK", connected = true))
        assertEquals("Paired with DESK", widgetLabel(running = true, pcName = "DESK", connected = false))
        assertEquals("Paired with DESK \u00b7 last seen 3 h ago", widgetLabel(true, "DESK", false, lastSeenMs = 1_000_000L, nowMs = 1_000_000L + 3 * 3600_000L))
        assertEquals("DESK not reachable \u00b7 last seen 3 h ago", widgetLabel(true, "DESK", false, lastSeenMs = 1_000_000L, nowMs = 1_000_000L + 3 * 3600_000L, attemptFailed = true))
        assertEquals("Starting\u2026", widgetLabel(running = false, pcName = null, connected = false, starting = true))
    }

    @Test
    fun connected_means_a_live_session_as_on_home() {
        assertFalse(widgetConnected(running = false, pc = "DESK", sessions = 1, controlledBy = null, screenTo = null, cameraTo = null))
        assertFalse(widgetConnected(running = true, pc = null, sessions = 1, controlledBy = null, screenTo = null, cameraTo = null))
        assertFalse(widgetConnected(running = true, pc = "DESK", sessions = 0, controlledBy = null, screenTo = null, cameraTo = null))
        assertTrue(widgetConnected(running = true, pc = "DESK", sessions = 1, controlledBy = null, screenTo = null, cameraTo = null))
        assertTrue(widgetConnected(running = true, pc = "DESK", sessions = 0, controlledBy = "DESK", screenTo = null, cameraTo = null))
        assertTrue(widgetConnected(running = true, pc = "DESK", sessions = 0, controlledBy = null, screenTo = "DESK", cameraTo = null))
        assertTrue(widgetConnected(running = true, pc = "DESK", sessions = 0, controlledBy = null, screenTo = null, cameraTo = "DESK"))
    }
}
