package dev.phonebridge.app

import android.app.KeyguardManager
import android.content.Context
import android.os.PowerManager

/** Why Android ended a screen share's projection. */
enum class StopReason { User, Locked }

/**
 * Android 15 QPR1 and later end a projection when the phone locks; Android's own "Stop sharing"
 * chip ends it too. Locked, or the screen off, means the lock did it; otherwise it was the user.
 */
fun projectionStopReason(keyguardLocked: Boolean, interactive: Boolean): StopReason =
    if (keyguardLocked || !interactive) StopReason.Locked else StopReason.User

fun projectionStopReason(ctx: Context): StopReason = projectionStopReason(
    keyguardLocked = ctx.getSystemService(KeyguardManager::class.java)?.isKeyguardLocked == true,
    interactive = ctx.getSystemService(PowerManager::class.java)?.isInteractive != false,
)
