package dev.phonebridge.app

import android.app.ActivityOptions
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.os.Build
import android.os.Bundle
import android.provider.Settings

/** `ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED` (API 34). */
private const val BAL_ALLOWED = 1

/** `ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOW_ALWAYS` (API 36; ALLOWED is deprecated there). */
private const val BAL_ALLOW_ALWAYS = 3

/** The background-start opt-in Relay gives a notification's PendingIntent when it sends it for the PC. */
fun bgStartMode(sdk: Int): Int? = when {
    sdk >= 36 -> BAL_ALLOW_ALWAYS
    sdk >= 34 -> BAL_ALLOWED
    else -> null
}

enum class OpenOutcome { SEND, TAP_ON_PHONE }

/**
 * The opt-in only helps if Relay itself may start activities from the background: with the overlay
 * permission or while Control's accessibility service is bound. Otherwise Android blocks the
 * app's screen silently, so the user is asked to tap a notification instead, and the PC is told.
 */
fun openOutcome(canStartActivity: Boolean): OpenOutcome =
    if (canStartActivity) OpenOutcome.SEND else OpenOutcome.TAP_ON_PHONE

/** What the PC shows when the phone needs a tap. */
const val TAP_ON_PHONE_TEXT = "Tap the notification on your phone"

internal object ActionSend {
    private const val TAP_ID = 15

    fun options(): Bundle? = bgStartMode(Build.VERSION.SDK_INT)?.let { mode ->
        if (Build.VERSION.SDK_INT >= 34) ActivityOptions.makeBasic().setPendingIntentBackgroundActivityStartMode(mode).toBundle() else null
    }

    fun canStartActivity(ctx: Context): Boolean = Settings.canDrawOverlays(ctx) || ControlService.instance != null

    /** A notification whose tap runs the app's own [open] intent: a tap is always allowed to start it. */
    fun postTap(ctx: Context, open: PendingIntent, what: CharSequence?) {
        val n = Notification.Builder(ctx, CH_EVENTS)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle("Tap to open")
            .setContentText(what?.toString()?.take(80) ?: "Your PC asked to open this on your phone")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        ctx.getSystemService(NotificationManager::class.java).notify(TAP_ID, n)
    }
}
