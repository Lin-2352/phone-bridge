package dev.phonebridge.app.ui.pairing

import android.content.Context
import android.os.SystemClock
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import dev.phonebridge.app.APP_NAME
import dev.phonebridge.app.AppLog
import dev.phonebridge.app.Bridge
import dev.phonebridge.app.PbService
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.pb_ffi.PbException

private const val TAG = "PhoneBridge"

/** Where pairing is, for the UI: nothing, contacting the PC (with a start time), or a failure to explain. */
sealed interface PairPhase {
    data object Idle : PairPhase
    data class Joining(val startedAt: Long) : PairPhase
    data class Failed(val text: String) : PairPhase
}

/**
 * Pairing, both steps, over the [Bridge] node: [start] joins with the scanned code and shows the code check
 * ([Bridge.sas]); [confirm] sends the user's answer. Unlike a bare call it can be cancelled, gives up after
 * [JOIN_LIMIT_MS] (longer than the transport's own dial and handshake waits, so the real error shows first),
 * ignores a second scan while one is running, and reports failures as sentences with a next step.
 */
object PairingFlow {
    /** Dial (15 s) plus the handshake (20 s) in pb-transport, and a little. */
    const val JOIN_LIMIT_MS = 40_000L

    val phase = mutableStateOf<PairPhase>(PairPhase.Idle)
    @Volatile private var job: Job? = null
    private val lock = Any()
    /** Bumped by every start and cancel: an attempt whose number is out of date has been given up. */
    private var gen = 0

    /** Join with a scanned or typed [code]; the progress shows at once. A second call while joining is ignored. */
    fun start(code: String) {
        synchronized(lock) {
            if (phase.value is PairPhase.Joining) return
            phase.value = PairPhase.Joining(SystemClock.elapsedRealtime())
            val g = ++gen
            job = Bridge.scope.launch { join(code, g) }
        }
    }

    /** The user gave up waiting: the attempt is dropped, and a code check that arrives later is refused. */
    fun cancel() {
        synchronized(lock) {
            if (phase.value !is PairPhase.Joining) return
            gen++
            job?.cancel()
            job = null
            phase.value = PairPhase.Idle
        }
        AppLog.event("events", "pair_step", "step" to "join_cancelled")
        // joinPairing may have finished just before the cancel: clear Rust's pending pairing and tell the PC no.
        Bridge.scope.launch { runCatching { Bridge.node?.confirmPairing(false) } }
    }

    fun dismiss() {
        phase.value = PairPhase.Idle
    }

    private suspend fun join(code: String, g: Int) {
        try {
            val node = Bridge.awaitNode()
            if (node == null) {
                fail("$APP_NAME is off. Turn it on, then scan again.")
                return
            }
            val sas = withTimeoutOrNull(JOIN_LIMIT_MS) { node.joinPairing(code.trim()) }
            synchronized(lock) {
                if (gen != g) {
                    // Cancelled while the call was finishing: refuse it so nothing stays half-paired.
                    if (sas != null) Bridge.scope.launch { runCatching { node.confirmPairing(false) } }
                    return
                }
            }
            if (sas == null) {
                AppLog.event("events", "pair_step", "step" to "join_failed", "type" to "Timeout")
                fail(joinErrorText("timed out"))
                return
            }
            Log.i(TAG, "SAS shown")
            AppLog.event("events", "pair_step", "step" to "code_shown")
            Bridge.sas.value = sas
            phase.value = PairPhase.Idle
        } catch (e: CancellationException) {
            throw e
        } catch (e: Throwable) {
            val reason = (e as? PbException.Failed)?.reason ?: e.message
            Log.i(TAG, "Pairing failed: ${e.javaClass.simpleName}")
            AppLog.event("events", "pair_step", "step" to "join_failed", "type" to e.javaClass.simpleName)
            if (synchronized(lock) { gen == g }) fail(joinErrorText(reason))
        }
    }

    private fun fail(text: String) {
        phase.value = PairPhase.Failed(text)
    }

    /** Step 2: the user compared the codes. */
    suspend fun confirm(ctx: Context, accept: Boolean) {
        Bridge.sas.value = null
        AppLog.event("events", "pair_step", "step" to if (accept) "confirmed" else "rejected")
        val n = Bridge.node ?: run { fail("Relay stopped. Turn it on and scan again."); return }
        try {
            val name = n.confirmPairing(accept)
            Log.i(TAG, "PAIRED with $name")
            AppLog.event("events", "pair_step", "step" to "paired")
            Bridge.linked.value = name
            // Pairing is contact too: Home says "Last seen just now", not "Not seen yet".
            Bridge.stampSeen(ctx)
            PbService.refresh(ctx)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Throwable) {
            val reason = (e as? PbException.Failed)?.reason ?: e.message
            Log.i(TAG, "Pairing not completed: ${e.javaClass.simpleName}")
            AppLog.event("events", "pair_step", "step" to "not_completed", "type" to e.javaClass.simpleName)
            // Saying No is not a failure: the dialog already went away, so no second one.
            if (accept) fail(confirmErrorText(true, reason))
        }
    }
}
