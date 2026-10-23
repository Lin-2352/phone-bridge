package dev.phonebridge.app

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.app.NotificationManager
import android.content.pm.PackageManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log

private const val TAG = "PhoneBridge"
private const val REQ_CAMERA = 8
/** The PC waits 40 s in all: our ask, then the permission and opening the camera. */
private const val ASK_MS = 30_000L

/**
 * "Let <PC> use your camera?" (shown briefly, or skipped, with "Always
 * allow"), then Android's camera permission if it is not held yet. The
 * service takes the camera type while this window is visible: Android lets
 * a foreground service open the camera only then.
 */
class CameraConsentActivity : Activity() {
    companion object {
        /** The window kept in front while the camera streams. */
        @Volatile private var current: CameraConsentActivity? = null

        /** The camera stopped: the window is not needed any more. */
        fun close() {
            current?.let { a -> a.runOnUiThread { a.finish() } }
        }
    }

    private var ask: CameraAsk? = null
    private var answered = false
    private var dialog: AlertDialog? = null
    private val main = Handler(Looper.getMainLooper())

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val i = intent
        val a = CameraAsk(
            id = i.getLongExtra("id", -1).toULong(),
            peer = i.getStringExtra("peer") ?: "Your PC",
            lens = i.getIntExtra("lens", 0),
            width = i.getIntExtra("width", 1920),
            height = i.getIntExtra("height", 1080),
            codecs = i.getIntArrayExtra("codecs")?.toList().orEmpty(),
        )
        ask = a
        getSystemService(NotificationManager::class.java).cancel(CAMERA_ASK_ID)
        val left = ASK_MS - (SystemClock.elapsedRealtime() - i.getLongExtra("at", SystemClock.elapsedRealtime()))
        if (left <= 0) {
            android.widget.Toast.makeText(this, "That request has expired", android.widget.Toast.LENGTH_SHORT).show()
            decline()
            return
        }
        if (Prefs(this).allowCamera) {
            ensurePermission()
            return
        }
        dialog = AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle("Let ${a.peer} use your camera?")
            .setMessage("Your camera becomes a webcam on the PC while it is on. Android shows its camera indicator, and Stop is in the notification.")
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
        if (always) Prefs(this).allowCamera = true
        ensurePermission()
    }

    private fun ensurePermission() {
        answered = true
        if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            begin()
        } else {
            requestPermissions(arrayOf(Manifest.permission.CAMERA), REQ_CAMERA)
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != REQ_CAMERA) return
        if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
            begin()
        } else {
            Log.i(TAG, "CAMERA permission denied")
            decline()
        }
    }

    /** Take the camera service type while this window is visible, then open the camera. */
    private fun begin() {
        val a = ask ?: return finish()
        if (PbService.instance?.enterCamera() != true) {
            Log.e(TAG, "CAMERA could not enter the camera service type")
            decline()
            return
        }
        if (!CameraShare.start(this, a)) {
            PbService.instance?.leaveCamera()
            finish()
            return
        }
        // Stay in front, invisible and touch-through, while the camera
        // streams: another camera app that was open (the phone's own) stays
        // paused underneath instead of taking the camera back ("higher
        // priority client"). It goes when the camera stops, or when the
        // user opens another app.
        window.addFlags(
            android.view.WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or
                android.view.WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE,
        )
        current = this
    }

    private fun decline() {
        answered = true
        main.removeCallbacksAndMessages(null)
        ask?.let { Bridge.node?.answerCamera(it.id, null) }
        Log.i(TAG, "CAMERA request declined")
        finish()
    }

    override fun onDestroy() {
        if (current === this) current = null
        main.removeCallbacksAndMessages(null)
        dialog?.setOnDismissListener(null)
        super.onDestroy()
    }
}
