package dev.phonebridge.app

/**
 * Banking apps refuse to open while an accessibility service is on, so the
 * control service switches itself off once a PC session that used it is over.
 * Never right after the user turned it on (no session yet), and never while
 * the PC still controls the phone or shares its screen.
 */
fun shouldSwitchOff(sessionSeen: Boolean, controlledBy: String?, sharingWith: String?): Boolean =
    sessionSeen && controlledBy == null && sharingWith == null
