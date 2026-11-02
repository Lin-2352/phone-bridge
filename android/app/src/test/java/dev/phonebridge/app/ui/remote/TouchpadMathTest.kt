package dev.phonebridge.app.ui.remote

import org.junit.Assert.assertEquals
import org.junit.Test

class TouchpadMathTest {
    @Test
    fun pointer_scale_unchanged_after_move() {
        // The numbers the pad shipped with before it moved out of TouchpadActivity.
        assertEquals(1.6f, POINTER_SCALE, 0f)
        assertEquals(1.1f, pointerGain(0f), 1e-6f)
        assertEquals(1.1f + 0.5f, pointerGain(6f), 1e-6f)
        assertEquals(2.1f, pointerGain(12f), 1e-6f)
        // The speed bonus caps at 1.4.
        assertEquals(2.5f, pointerGain(1000f), 1e-6f)
    }
}
