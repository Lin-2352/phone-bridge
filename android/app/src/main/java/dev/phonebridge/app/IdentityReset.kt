package dev.phonebridge.app

import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * "Reset and pair again" (M3.7, B5): the Keystore can no longer open this phone's identity. Removes only what
 * `resetIdentity` removes (the sealed identity and the paired PCs it signed for), the Keystore key and what
 * points at the old PCs; never the rest of the data folder. Relay then starts with a new identity and the
 * scanner opens to pair again.
 */
fun resetIdentity(ctx: Context) {
    val app = ctx.applicationContext
    setBridgeEnabled(app, false)
    runCatching { uniffi.pb_ffi.resetIdentity(app.filesDir.absolutePath) }
        .onFailure { Log.e("PhoneBridge", "IDENTITY reset failed: ${it.javaClass.simpleName}") }
    KeystoreWrapper.forget()
    val p = Prefs(app)
    p.dropAllowancesExcept(emptyList())
    p.lastPcName = null
    p.lastSeenMs = 0L
    p.identityLost = false
    AppLog.event("events", "identity_reset")
    setBridgeEnabled(app, true)
    app.startActivity(Intent(app, ScanActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}

const val IDENTITY_LOST_ID = 16

/** One notification when the identity can't be opened; its tap opens Home, where the reset is offered. */
fun notifyIdentityLost(ctx: Context) {
    val nm = ctx.getSystemService(android.app.NotificationManager::class.java)
    nm.createNotificationChannel(
        android.app.NotificationChannel("events", "Files and clipboard", android.app.NotificationManager.IMPORTANCE_DEFAULT),
    )
    val open = android.app.PendingIntent.getActivity(
        ctx, 60, Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        android.app.PendingIntent.FLAG_IMMUTABLE,
    )
    val n = android.app.Notification.Builder(ctx, "events")
        .setSmallIcon(R.drawable.ic_stat_bridge)
        .setContentTitle("$APP_NAME can't open its keys")
        .setContentText("Tap to reset and pair again with your PC")
        .setContentIntent(open)
        .setAutoCancel(true)
        .build()
    nm.notify(IDENTITY_LOST_ID, n)
}
