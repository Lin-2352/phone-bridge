package dev.phonebridge.app

import dev.phonebridge.app.security.gated
import dev.phonebridge.app.security.Guarded
import android.Manifest
import android.app.NotificationManager
import android.content.Intent
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import androidx.fragment.app.FragmentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.phonebridge.app.audio.AudioReason
import dev.phonebridge.app.audio.AudioSource
import dev.phonebridge.app.audio.CaptureKind
import dev.phonebridge.app.audio.ConsentStep
import dev.phonebridge.app.audio.chooseCapture
import dev.phonebridge.app.audio.consentSteps
import dev.phonebridge.app.ui.RelayTheme

private const val TAG = "PhoneBridgeAudio"
private const val REQ_CAPTURE = 9
private const val REQ_MIC = 10

/** The PC waits 40 s in all; Rust gives the whole question 35 s. */
private const val ASK_MS = 30_000L

/**
 * "Let <PC> hear ...?" then, as needed, the RECORD_AUDIO grant and Android's capture dialog.
 * Microphone: Relay's dialog is shown every time, no setting skips it. Playback: Relay's dialog is
 * skipped by the "don't ask" setting, and Android's capture dialog is only skipped when the audio
 * helper does the capture, or when the user allowed it outside Relay.
 */
class AudioConsentActivity : FragmentActivity() {
    private var ask: AudioAsk? = null
    private val steps = ArrayDeque<ConsentStep>()
    private var finished = false
    private var useHelper = false
    private var asking by mutableStateOf(false)
    private val main = Handler(Looper.getMainLooper())

    override fun dispatchTouchEvent(ev: android.view.MotionEvent): Boolean =
        if (isObscured(ev.flags)) true else super.dispatchTouchEvent(ev)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        SecureWindow.apply(this)
        lifecycle.addObserver(ConsentGuard.observer("AudioConsentActivity"))
        val i = intent
        val source = if (i.getIntExtra("source", 0) == 1) AudioSource.MICROPHONE else AudioSource.PLAYBACK
        val a = AudioAsk(
            id = i.getLongExtra("id", -1).toULong(),
            peer = i.getStringExtra("peer") ?: "Your PC",
            source = source,
            sampleRate = i.getIntExtra("rate", 48000),
            channels = i.getIntExtra("channels", 2),
        )
        ask = a
        live = this
        getSystemService(NotificationManager::class.java).cancel(AUDIO_ASK_ID)
        val at = i.getLongExtra("at", SystemClock.elapsedRealtime())
        val left = ASK_MS - (SystemClock.elapsedRealtime() - at)
        if (left <= 0) {
            android.widget.Toast.makeText(this, "That request has expired", android.widget.Toast.LENGTH_SHORT).show()
            refuse(AudioReason.DECLINED)
            return
        }
        val has = checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
        val prefs = Prefs(this)
        useHelper = chooseCapture(source, prefs.audioSilencePhone, HelperAudio.alive(this)) == CaptureKind.HELPER
        // The helper needs neither Android's prompt nor the recording permission: only Relay's own question remains.
        val all = consentSteps(source, dev.phonebridge.app.devices.consentStep(prefs.pcAllow(intent.getStringExtra("peerId").orEmpty(), dev.phonebridge.app.devices.ConsentKind.Audio), keyguardLocked()) == dev.phonebridge.app.devices.ConsentStep.AutoAllow, has || useHelper)
        steps.addAll(if (useHelper) all.filter { it != ConsentStep.PROJECTION } else all)
        next()
        main.postDelayed({ if (!finished && asking) refuse(AudioReason.DECLINED) }, left)
    }

    private fun next() {
        val a = ask ?: return
        when (steps.removeFirstOrNull()) {
            ConsentStep.RELAY_DIALOG -> showDialog(a)
            ConsentStep.RECORD_AUDIO -> requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), REQ_MIC)
            ConsentStep.PROJECTION -> {
                val mpm = getSystemService(MediaProjectionManager::class.java)
                runCatching { startActivityForResult(captureIntent(mpm), REQ_CAPTURE) }
                    .onFailure {
                        Log.e(TAG, "AUDIO capture prompt failed: $it")
                        refuse(AudioReason.UNAVAILABLE)
                    }
            }
            null -> when {
                useHelper -> startHelper(a)
                a.source == AudioSource.MICROPHONE -> startCapture(null)
                else -> refuse(AudioReason.UNAVAILABLE)
            }
        }
    }

    private fun showDialog(a: AudioAsk) {
        asking = true
        val mic = a.source == AudioSource.MICROPHONE
        setContent {
            RelayTheme {
                if (asking) {
                    ConsentDialog(
                        title = if (mic) "Let ${a.peer} hear your microphone?" else "Let ${a.peer} hear this phone's audio?",
                        body = if (mic) {
                            "Your microphone is recorded and played on the PC for as long as sharing runs. $APP_NAME asks every time. Stop any time from the notification."
                        } else {
                            if (useHelper) {
                                "Whatever this phone plays is sent to the PC and plays there, and this phone stays silent. Stop any time from the notification."
                            } else {
                                "Whatever this phone plays is sent to the PC and plays there. Android asks once more before it starts. Stop any time from the notification."
                            }
                        },
                        onAllow = { gated(Guarded.AllowDecision, "Allow audio sharing") { allow() } },
                        onDeny = { if (!finished) refuse(AudioReason.DECLINED) },
                    )
                }
            }
        }
    }

    private fun allow() {
        if (finished) return
        asking = false
        next()
    }

    @Deprecated("Activity result API is not used in this app")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQ_CAPTURE) return
        val a = ask ?: return finish()
        if (resultCode != RESULT_OK || data == null) {
            refuse(AudioReason.DECLINED)
            return
        }
        // Android 14: the service must be in the mediaProjection type before the token is used.
        val reason = AudioShare.prepare(a)
        if (reason != null) {
            refuse(reason)
            return
        }
        val projection = getSystemService(MediaProjectionManager::class.java).getMediaProjection(resultCode, data)
        if (projection == null) {
            refuse(AudioReason.UNAVAILABLE, teardown = true)
            return
        }
        finished = true
        main.removeCallbacksAndMessages(null)
        AudioShare.launch(this, a, projection)
        finish()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != REQ_MIC) return
        if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) next() else refuse(AudioReason.NO_PERMISSION)
    }

    /** Microphone path: nothing more to ask. The service type needs the grant, so [AudioShare.prepare] runs here. */
    private fun startCapture(projection: android.media.projection.MediaProjection?) {
        val a = ask ?: return finish()
        val reason = AudioShare.prepare(a)
        if (reason != null) {
            refuse(reason)
            return
        }
        finished = true
        main.removeCallbacksAndMessages(null)
        AudioShare.launch(this, a, projection)
        finish()
    }

    private fun startHelper(a: AudioAsk) {
        val reason = AudioShare.prepareHelper()
        if (reason != null) {
            refuse(reason)
            return
        }
        finished = true
        main.removeCallbacksAndMessages(null)
        AudioShare.launch(this, a, null, helper = true)
        finish()
    }

    /** [teardown]: this request had claimed the direction ([AudioShare.prepare] succeeded). */
    private fun refuse(reason: AudioReason, teardown: Boolean = false) {
        if (finished) return
        finished = true
        asking = false
        main.removeCallbacksAndMessages(null)
        ask?.let { AudioShare.abort(it, reason, teardown) }
        Log.i(TAG, "AUDIO consent ended without sharing: $reason")
        finish()
    }

    override fun onDestroy() {
        main.removeCallbacksAndMessages(null)
        if (live === this) live = null
        // Closed by the system before an answer: the PC must not wait the full time.
        if (!finished) {
            finished = true
            ask?.let { AudioShare.abort(it, AudioReason.DECLINED, teardown = false) }
        }
        super.onDestroy()
    }

    companion object {
        @Volatile private var live: AudioConsentActivity? = null

        /** The question [id] is over (timeout, PC left): close the window. */
        fun closeFor(id: ULong) {
            val a = live ?: return
            if (a.ask?.id != id) return
            a.finished = true
            a.runOnUiThread { a.finish() }
        }
    }
}
