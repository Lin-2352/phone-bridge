package dev.phonebridge.app

import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import dev.phonebridge.app.security.GateAvailability
import dev.phonebridge.app.security.GateResult
import dev.phonebridge.app.security.authenticators
import dev.phonebridge.app.security.canEnableGate
import dev.phonebridge.app.security.classifyBiometricError
import dev.phonebridge.app.security.gateAvailability
import dev.phonebridge.app.security.gateDefault
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class GateTest {
    @Test
    fun api30_uses_strong_or_credential() {
        assertEquals(
            BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL,
            authenticators(30),
        )
    }

    @Test
    fun api29_uses_weak_or_credential() {
        assertEquals(
            BiometricManager.Authenticators.BIOMETRIC_WEAK or BiometricManager.Authenticators.DEVICE_CREDENTIAL,
            authenticators(29),
        )
    }

    @Test
    fun cannot_enable_without_a_screen_lock() {
        assertFalse(canEnableGate(deviceSecure = false))
        assertTrue(canEnableGate(deviceSecure = true))
    }

    @Test
    fun default_on_in_release_and_qa_off_in_debug() {
        assertTrue(gateDefault(debugBuild = false))
        assertFalse(gateDefault(debugBuild = true))
    }

    @Test
    fun gate_on_without_a_screen_lock_is_unavailable_with_a_reason() {
        assertEquals(GateAvailability.Active, gateAvailability(enabled = true, deviceSecure = true))
        assertEquals(GateAvailability.Off, gateAvailability(enabled = false, deviceSecure = true))
        assertEquals(GateAvailability.NoScreenLock, gateAvailability(enabled = true, deviceSecure = false))
    }

    @Test
    fun biometric_lockout_maps_to_failed_never_unavailable() {
        assertEquals(GateResult.Failed, classifyBiometricError(BiometricPrompt.ERROR_LOCKOUT))
        assertEquals(GateResult.Failed, classifyBiometricError(BiometricPrompt.ERROR_LOCKOUT_PERMANENT))
        assertEquals(GateResult.Cancelled, classifyBiometricError(BiometricPrompt.ERROR_USER_CANCELED))
        assertEquals(GateResult.Unavailable, classifyBiometricError(BiometricPrompt.ERROR_NO_BIOMETRICS))
    }
}
