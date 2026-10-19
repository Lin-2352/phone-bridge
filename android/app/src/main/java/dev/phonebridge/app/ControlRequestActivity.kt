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
    private val main = Handler(Looper.getMainLooper())

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        id = intent.getLongExtra("id", -1)
        val peer = intent.getStringExtra("peer") ?: "Your PC"
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle("Let $peer control this phone?")
            .setMessage("Its mouse and keyboard will work here. Stop any time from Phone Bridge or with Ctrl+Alt+P on the PC.")
            .setPositiveButton("Allow") { _, _ -> answer(true, always = false) }
            .setNeutralButton("Always allow") { _, _ -> answer(true, always = true) }
            .setNegativeButton("Deny") { _, _ -> answer(false, always = false) }
            .setOnDismissListener { if (!answered) answer(false, always = false) }
            .show()
        // The PC stops waiting after 30 s.
        main.postDelayed({ if (!answered) answer(false, always = false) }, 30_000)
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
        super.onDestroy()
    }
}
