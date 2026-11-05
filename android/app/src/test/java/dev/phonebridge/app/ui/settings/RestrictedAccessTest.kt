package dev.phonebridge.app.ui.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RestrictedAccessTest {
    @Test
    fun androidTwelveHasNoRestrictedSettings() {
        assertFalse(restrictedAccessLikely(sdk = 32, installer = "com.android.chrome"))
    }

    @Test
    fun appsFromThePlayStoreAreNotRestricted() {
        assertFalse(restrictedAccessLikely(sdk = 36, installer = "com.android.vending"))
    }

    @Test
    fun anythingElseFromAndroidThirteenOnMayBeRestricted() {
        assertTrue(restrictedAccessLikely(sdk = 33, installer = "com.android.chrome"))
        assertTrue(restrictedAccessLikely(sdk = 36, installer = "com.google.android.documentsui"))
        assertTrue(restrictedAccessLikely(sdk = 34, installer = null))
    }

    @Test
    fun stepsNameTheFeatureAndKeepAndroidsOrder() {
        val steps = restrictedAccessSteps(SpecialAccess.Notifications)
        assertEquals(3, steps.size)
        assertTrue(steps[0], steps[0].contains("Notification access"))
        assertTrue(steps[1], steps[1].contains("Allow restricted settings"))
        assertTrue(steps[2], steps[2].contains("Notification access"))
    }

    @Test
    fun stepsUseTheNameAndroidShowsForEachEntry() {
        assertTrue(restrictedAccessSteps(SpecialAccess.Control)[0].contains("Accessibility and try to turn on Relay control"))
        assertTrue(restrictedAccessSteps(SpecialAccess.Notifications)[0].contains("turn on Relay."))
    }

    @Test
    fun onceAServiceHasConnectedTheStepsAreNotShownAgain() {
        // Control switches itself off after each session, so "off now" doesn't mean locked.
        assertTrue(showRestrictedSteps(sdk = 36, installer = null, serviceEverConnected = false))
        assertFalse(showRestrictedSteps(sdk = 36, installer = null, serviceEverConnected = true))
        assertFalse(showRestrictedSteps(sdk = 36, installer = "com.android.vending", serviceEverConnected = false))
    }
}
