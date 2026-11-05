package dev.phonebridge.app

/**
 * Banking apps refuse to open while an accessibility service is on, so the
 * control service switches itself off once a PC session that used it is over.
 * Never right after the user turned it on (no session yet), and never while
 * the PC still controls the phone or shares its screen. The user may keep it on
 * ([Prefs.keepControlOn]); remote input still pauses while the phone is locked.
 */
fun shouldSwitchOff(keepOn: Boolean, sessionSeen: Boolean, controlledBy: String?, sharingWith: String?, holdingVolumeKeys: Boolean = false): Boolean =
    !keepOn && sessionSeen && controlledBy == null && sharingWith == null && !holdingVolumeKeys
