package dev.phonebridge.app.security

import android.os.Build
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.lifecycleScope
import kotlin.coroutines.resume
import kotlinx.coroutines.launch
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

/** What a guarded action goes through, so the gate and its grace can be tested without a screen. */
enum class Guarded { AllowDecision, AlwaysAllow, AllowListChange, AutoAccept, ConfirmPairing, Unpair, GateOff }

/** What to do for a guarded action. */
enum class GateStep {
    /** Go ahead: the gate is off, or the person unlocked a moment ago. */
    Proceed,

    /** Ask for the unlock first. */
    Prompt,

    /** The gate is on but cannot run (the screen lock was removed): the old consent still stands. */
    ProceedNoGate,
}

fun gateStep(enabled: Boolean, available: Boolean, graceOk: Boolean): GateStep = when {
    !enabled -> GateStep.Proceed
    !available -> GateStep.ProceedNoGate
    graceOk -> GateStep.Proceed
    else -> GateStep.Prompt
}

/**
 * After a successful unlock nothing asks again for [windowMs]. A failed or cancelled attempt starts no grace;
 * locking the screen resets it; a clock that moved backwards cannot prove it, so that asks too.
 */
class Grace(private val windowMs: Long = 60_000, private val clock: () -> Long = { android.os.SystemClock.elapsedRealtime() }) {
    private var last: Long? = null

    fun needsUnlock(): Boolean {
        val at = last ?: return true
        val now = clock()
        return now < at || now - at >= windowMs
    }

    fun passed() {
        last = clock()
    }

    fun reset() {
        last = null
    }
}

/** The process-wide grace, and the receiver that resets it when the screen locks. */
object GateState {
    val grace = Grace()
    private var receiverRegistered = false

    fun ensureLockReceiver(app: android.content.Context) {
        if (receiverRegistered) return
        receiverRegistered = true
        app.registerReceiver(
            object : android.content.BroadcastReceiver() {
                override fun onReceive(c: android.content.Context, i: android.content.Intent) {
                    grace.reset()
                }
            },
            android.content.IntentFilter(android.content.Intent.ACTION_SCREEN_OFF),
        )
    }
}

/**
 * Runs the gate for a guarded action: true when the action may go ahead (the gate is off or cannot run, the
 * grace holds, or the person just unlocked), false when they did not unlock. A failed or cancelled unlock leaves
 * the action undone.
 */
suspend fun FragmentActivity.requireUnlock(what: Guarded, reason: String): Boolean {
    GateState.ensureLockReceiver(applicationContext)
    val secure = getSystemService(android.app.KeyguardManager::class.java)?.isDeviceSecure == true
    val prefs = dev.phonebridge.app.Prefs(this)
    return when (gateStep(prefs.unlockGate, secure, !GateState.grace.needsUnlock())) {
        GateStep.Proceed, GateStep.ProceedNoGate -> true
        GateStep.Prompt -> when (unlock("$reason (${what.name})")) {
            GateResult.Passed -> {
                GateState.grace.passed()
                true
            }
            else -> false
        }
    }
}

/** Runs [block] once the gate lets the guarded action through; a failed or cancelled unlock leaves it undone. */
fun FragmentActivity.gated(what: Guarded, reason: String, block: () -> Unit) {
    lifecycleScope.launch {
        if (requireUnlock(what, reason)) block()
    }
}
