package dev.phonebridge.app

import android.app.KeyguardManager
import android.content.Context

/** "Always allow" (control, screen, camera, audio) never applies while the phone is locked (S4). */
fun shouldAutoAccept(alwaysAllowed: Boolean, keyguardLocked: Boolean): Boolean = alwaysAllowed && !keyguardLocked

/** Remote input reaches the phone only while it is unlocked and none of Relay's prompts is on screen. */
fun inputAllowed(keyguardLocked: Boolean, consentActive: Boolean): Boolean = !keyguardLocked && !consentActive

/** Before Android 12 a notification action can't require an unlock, so "Allow" opens the question instead. */
fun allowActionOpensActivity(sdkInt: Int): Boolean = sdkInt < 31

fun Context.keyguardLocked(): Boolean = getSystemService(KeyguardManager::class.java)?.isKeyguardLocked == true
