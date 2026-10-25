package dev.phonebridge.app

import android.app.Activity
import android.app.AlertDialog
import android.app.NotificationManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log

private const val TAG = "PhoneBridge"

/**
 * "Let <PC> control this phone?" Shown when the PC asks for control and the
 * user has not set "Let my PC control this phone". Allow once, always allow,
 * or deny; no answer within 30 s counts as deny (the request times out).
 */
class ControlRequestActivity : Activity() {
    private var answered = false
    private var id = -1L
    private var dialog: AlertDialog? = null
    private val main = Handler(Looper.getMainLooper())

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        current = this
        show(intent)
    }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        // Only one request can be pending; a new id means the old one ended.
        main.removeCallbacksAndMessages(null)
        dialog?.setOnDismissListener(null)
        dialog?.dismiss()
        answered = false
        show(intent)
    }

    private fun show(intent: android.content.Intent) {
        id = intent.getLongExtra("id", -1)
        val peer = intent.getStringExtra("peer") ?: "Your PC"
        // The phone stops waiting 30 s after the request arrived, not after
        // this window opened (it may be opened late from the notification).
        val left = 30_000 - (android.os.SystemClock.elapsedRealtime() - intent.getLongExtra("at", android.os.SystemClock.elapsedRealtime()))
        if (left <= 0) {
            android.widget.Toast.makeText(this, "That request has expired", android.widget.Toast.LENGTH_SHORT).show()
            finish()
            return
        }
        dialog = AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle("Let $peer control this phone?")
            .setMessage("Its mouse and keyboard will work here. Stop any time from $APP_NAME or with Ctrl+Alt+P on the PC.")
            .setPositiveButton("Allow") { _, _ -> answer(true, always = false) }
            .setNeutralButton("Always allow") { _, _ -> answer(true, always = true) }
            .setNegativeButton("Deny") { _, _ -> answer(false, always = false) }
            .setOnDismissListener { if (!answered) answer(false, always = false) }
            .show()
        main.postDelayed({ if (!answered) answer(false, always = false) }, left)
    }

    private fun answer(accept: Boolean, always: Boolean) {
        if (answered) return
        answered = true
        if (always) Bridge.setAllowControl(this, true)
        Bridge.node?.answerOffer(id.toULong(), accept)
        getSystemService(NotificationManager::class.java).cancel(CONTROL_ASK_ID)
        Log.i(TAG, "CONTROL request answered accept=$accept always=$always")
        finish()
    }

    override fun onDestroy() {
        main.removeCallbacksAndMessages(null)
        if (current === this) current = null
        super.onDestroy()
    }

    companion object {
        @Volatile private var current: ControlRequestActivity? = null

        /** The same request was answered from the notification. */
        fun answeredElsewhere(id: Long) {
            val a = current ?: return
            a.main.post {
                if (a.id == id && !a.answered) {
                    a.answered = true
                    a.finish()
                }
            }
        }
    }
}
