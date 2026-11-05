package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class StopReasonTest {
    @Test fun lockOrScreenOffIsLocked() {
        assertEquals(StopReason.Locked, projectionStopReason(keyguardLocked = true, interactive = true))
        assertEquals(StopReason.Locked, projectionStopReason(keyguardLocked = false, interactive = false))
    }

    /** Android's own "Stop sharing" chip, tapped on an unlocked phone, is still the user's Stop. */
    @Test fun unlockedStopIsUser() =
        assertEquals(StopReason.User, projectionStopReason(keyguardLocked = false, interactive = true))
}
