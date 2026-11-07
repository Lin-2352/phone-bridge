package dev.phonebridge.app.security

import android.os.Build
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine

/** The unlock gate's state for the settings row. */
enum class GateAvailability { Active, Off, NoScreenLock }

/** How an unlock attempt ended. */
enum class GateResult { Passed, Failed, Cancelled, Unavailable }

/** Authenticators allowed by the gate: a strong biometric or the device credential (weak on API 29). */
fun authenticators(sdk: Int): Int =
    if (sdk >= 30) {
        BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL
    } else {
        BiometricManager.Authenticators.BIOMETRIC_WEAK or BiometricManager.Authenticators.DEVICE_CREDENTIAL
    }

/** The gate needs a screen lock: without one there is nothing to unlock with. */
fun canEnableGate(deviceSecure: Boolean): Boolean = deviceSecure

/** On in release and qa builds, off in debug builds (where a prompt on every action would get in the way). */
fun gateDefault(debugBuild: Boolean): Boolean = !debugBuild

fun gateAvailability(enabled: Boolean, deviceSecure: Boolean): GateAvailability = when {
    !enabled -> GateAvailability.Off
    !deviceSecure -> GateAvailability.NoScreenLock
    else -> GateAvailability.Active
}

/** Maps a BiometricPrompt error to the gate's result. A lockout is a failure, never "unavailable". */
fun classifyBiometricError(errorCode: Int): GateResult = when (errorCode) {
    BiometricPrompt.ERROR_LOCKOUT, BiometricPrompt.ERROR_LOCKOUT_PERMANENT -> GateResult.Failed
    BiometricPrompt.ERROR_USER_CANCELED, BiometricPrompt.ERROR_NEGATIVE_BUTTON, BiometricPrompt.ERROR_CANCELED ->
        GateResult.Cancelled
    BiometricPrompt.ERROR_NO_BIOMETRICS, BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL,
    BiometricPrompt.ERROR_HW_NOT_PRESENT, BiometricPrompt.ERROR_HW_UNAVAILABLE -> GateResult.Unavailable
    else -> GateResult.Failed
}

/** Asks the person to unlock (their fingerprint, face or screen lock). There is no negative button: a credential is allowed. */
suspend fun FragmentActivity.unlock(reason: String): GateResult =
    suspendCancellableCoroutine { cont ->
        val prompt = BiometricPrompt(
            this,
            ContextCompat.getMainExecutor(this),
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    if (cont.isActive) cont.resume(GateResult.Passed)
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    if (cont.isActive) cont.resume(classifyBiometricError(errorCode))
                }
            },
        )
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle("Unlock Relay")
            .setSubtitle(reason)
            .setAllowedAuthenticators(authenticators(Build.VERSION.SDK_INT))
            .build()
        prompt.authenticate(info)
        cont.invokeOnCancellation { prompt.cancelAuthentication() }
    }
