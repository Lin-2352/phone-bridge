package dev.phonebridge.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LocalNetworkTest {
    @Test fun noRequestWhileTargeting36() = assertFalse(needsLocalNetworkRequest(sdkInt = 37, targetSdk = 36, granted = false))

    @Test fun requestOn37WhenTargeting37AndNotGranted() {
        assertTrue(needsLocalNetworkRequest(sdkInt = 37, targetSdk = 37, granted = false))
        assertFalse(needsLocalNetworkRequest(sdkInt = 37, targetSdk = 37, granted = true))
        assertFalse(needsLocalNetworkRequest(sdkInt = 36, targetSdk = 37, granted = false))
    }
}
