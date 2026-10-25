package dev.phonebridge.app

import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import android.app.NotificationManager
import android.content.Intent
import android.media.projection.MediaProjectionManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log

private const val TAG = "PhoneBridge"
private const val REQ_CAPTURE = 7
/** The PC waits 40 s in all: our ask, then Android's capture prompt. */
private const val ASK_MS = 30_000L

/**
 * "Let <PC> see this screen?" (skipped with "Always allow"), then Android's
 * own capture prompt, which Android shows every session. On success the
 * capture starts and the PC gets its answer.
 */
class ScreenConsentActivity : ComponentActivity() {
    private var ask: ScreenAsk? = null
    private var answered = false
    private var asking by mutableStateOf(false)
    private var askedAt = 0L
    private var promptAt = 0L
    private val main = Handler(Looper.getMainLooper())

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val i = intent
        val a = ScreenAsk(
            id = i.getLongExtra("id", -1).toULong(),
            peer = i.getStringExtra("peer") ?: "Your PC",
            maxLong = i.getIntExtra("maxLong", 1920),
            sharp = i.getBooleanExtra("sharp", false),
            dim = i.getBooleanExtra("dim", false),
            codecs = i.getIntArrayExtra("codecs")?.toList().orEmpty(),
        )
        ask = a
        askedAt = i.getLongExtra("at", SystemClock.elapsedRealtime())
        Log.i(TAG, "SCREEN ask window open after ${SystemClock.elapsedRealtime() - askedAt} ms")
        getSystemService(NotificationManager::class.java).cancel(SCREEN_ASK_ID)
        val left = ASK_MS - (SystemClock.elapsedRealtime() - i.getLongExtra("at", SystemClock.elapsedRealtime()))
        if (left <= 0) {
            android.widget.Toast.makeText(this, "That request has expired", android.widget.Toast.LENGTH_SHORT).show()
            decline()
            return
        }
        if (Prefs(this).allowScreen) {
            capturePrompt()
            return
        }
        if (ScreenShare.armed != null) Log.i(TAG, "SCREEN instant share waiting for the user's yes")
        asking = true
        setContent {
            PbTheme {
                if (asking) {
                    ConsentDialog(
                        title = "Let ${a.peer} see this screen?",
                        body = "Your screen appears in a window on the PC, which can also tap and type here. Stop any time from $APP_NAME or the notification.",
                        onAllow = { allow(always = false) },
                        onDeny = { if (!answered) decline() },
                        onAlways = { allow(always = true) },
                    )
                }
            }
        }
        main.postDelayed({ if (!answered) decline() }, left)
    }

    private fun allow(always: Boolean) {
        if (answered) return
        answered = true
        asking = false
        main.removeCallbacksAndMessages(null)
        if (always) Prefs(this).allowScreen = true
        capturePrompt()
    }

    private fun capturePrompt() {
        answered = true
        // Instant screen: the capture is already allowed and running idle.
        ask?.let { a ->
            if (ScreenShare.armed != null) {
                if (!ScreenShare.startArmed(a)) Bridge.node?.answerScreen(a.id, null)
                finish()
                return
            }
        }
        promptAt = SystemClock.elapsedRealtime()
        Log.i(TAG, "SCREEN capture prompt after ${promptAt - askedAt} ms")
        val mpm = getSystemService(MediaProjectionManager::class.java)
        runCatching { startActivityForResult(mpm.createScreenCaptureIntent(), REQ_CAPTURE) }
            .onFailure {
                Log.e(TAG, "SCREEN capture prompt failed: $it")
                decline()
            }
    }

    @Deprecated("Activity result API is not used in this app")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQ_CAPTURE) return
        val a = ask ?: return finish()
        Log.i(TAG, "SCREEN capture prompt answered after ${SystemClock.elapsedRealtime() - promptAt} ms")
        if (resultCode != RESULT_OK || data == null) {
            Log.i(TAG, "SCREEN capture prompt cancelled")
            decline()
            return
        }
        // Android 14+: the service must run as mediaProjection before the
        // projection is created (allowed now that the user agreed).
        if (PbService.instance?.enterProjection() != true) {
            Log.e(TAG, "SCREEN could not enter the mediaProjection service type")
            decline()
            return
        }
        val projection = getSystemService(MediaProjectionManager::class.java).getMediaProjection(resultCode, data)
        if (projection == null || !ScreenShare.start(this, projection, a)) {
            PbService.instance?.leaveProjection()
            if (projection == null) Bridge.node?.answerScreen(a.id, null)
        }
        finish()
    }

    private fun decline() {
        answered = true
        asking = false
        main.removeCallbacksAndMessages(null)
        ask?.let { Bridge.node?.answerScreen(it.id, null) }
        Log.i(TAG, "SCREEN request declined")
        finish()
    }

    override fun onDestroy() {
        main.removeCallbacksAndMessages(null)
        super.onDestroy()
    }
}
