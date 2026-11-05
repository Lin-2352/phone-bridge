package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class CaptureConfigTest {
    @Test
    fun full_display_from_api_34() {
        // Android 14 lets the user pick one app; clicks would then land at full-screen coordinates.
        assertEquals(CaptureConfig.DEFAULT_DISPLAY, captureConfig(34))
        assertEquals(CaptureConfig.DEFAULT_DISPLAY, captureConfig(36))
    }

    @Test
    fun no_config_below_34() {
        assertEquals(CaptureConfig.NONE, captureConfig(33))
        assertEquals(CaptureConfig.NONE, captureConfig(29))
    }
}
