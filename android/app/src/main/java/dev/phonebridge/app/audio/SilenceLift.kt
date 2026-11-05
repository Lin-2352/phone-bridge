package dev.phonebridge.app.audio

/** `AudioAttributes.USAGE_ALARM`; emergency alerts play with it too. */
private const val USAGE_ALARM = 4

/** `AudioAttributes.USAGE_NOTIFICATION_RINGTONE`. */
private const val USAGE_RINGTONE = 6

/**
 * The speaker silencer sits on the shared output mix. On phones where alarms, ringtones or
 * emergency alerts play through that same output, it must step aside while one plays.
 */
fun mustLiftSilencer(usages: List<Int>): Boolean = usages.any { it == USAGE_ALARM || it == USAGE_RINGTONE }
