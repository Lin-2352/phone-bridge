package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class AskTextTest {
    @Test
    fun lockedPhoneAsksToUnlockFirst() {
        assertEquals("Unlock to use your camera for My PC", askTitle("use your camera", "My PC", locked = true))
        assertEquals("Then tap here to allow it", askBody(locked = true))
    }

    @Test
    fun unlockedPhoneShowsTheRequestAsAQuestion() {
        // Shown when Android blocks the window from opening in the background.
        assertEquals("My PC wants to use your camera", askTitle("use your camera", "My PC", locked = false))
        assertEquals("Tap here to answer", askBody(locked = false))
    }
}
