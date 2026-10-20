package dev.phonebridge.app

import android.app.Activity
import android.app.AlertDialog
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
class ScreenConsentActivity : Activity() {
    private var ask: ScreenAsk? = null
    private var answered = false
    private var dialog: AlertDialog? = null
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
        dialog = AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle("Let ${a.peer} see this screen?")
            .setMessage("Your screen appears in a window on the PC, which can also tap and type here. Stop any time from Phone Bridge or the notification.")
            .setPositiveButton("Allow") { _, _ -> allow(always = false) }
            .setNeutralButton("Always allow") { _, _ -> allow(always = true) }
            .setNegativeButton("Deny") { _, _ -> decline() }
            .setOnDismissListener { if (!answered) decline() }
            .show()
        main.postDelayed({ if (!answered) decline() }, left)
    }

    private fun allow(always: Boolean) {
        if (answered) return
        answered = true
        main.removeCallbacksAndMessages(null)
        if (always) Prefs(this).allowScreen = true
        capturePrompt()
    }

    private fun capturePrompt() {
        answered = true
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
        main.removeCallbacksAndMessages(null)
        ask?.let { Bridge.node?.answerScreen(it.id, null) }
        Log.i(TAG, "SCREEN request declined")
        finish()
    }

    override fun onDestroy() {
        main.removeCallbacksAndMessages(null)
        dialog?.setOnDismissListener(null)
        super.onDestroy()
    }
}
