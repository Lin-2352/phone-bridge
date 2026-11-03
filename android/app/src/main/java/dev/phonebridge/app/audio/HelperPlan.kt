package dev.phonebridge.app.audio

import dev.phonebridge.helper.HelperProtocol

/** After a failed start, wait this long before trying by itself. */
const val HELPER_RETRY_MS = 5 * 60_000L

/** Android forgets an adb key after about 7 days unused, so Relay connects well before that. */
const val HELPER_RENEW_MS = 5L * 24 * 60 * 60 * 1000

private const val MIN_SDK = 31
private const val CODE_DIGITS = 6

/** What is true about the helper right now. */
data class HelperFacts(
    val paired: Boolean,
    val sdk: Int,
    val wifi: Boolean,
    val alive: Boolean,
    /** Holds the grant that lets Relay turn Wireless debugging on by itself. */
    val canToggle: Boolean,
    val msSinceAttempt: Long,
)

enum class HelperStep { NOTHING, START, TOO_SOON, NO_WIFI, NEEDS_SETUP_AGAIN, NOT_SET_UP, UNSUPPORTED }

/** [asked]: a PC is asking right now, which is worth a try even just after a failure. */
fun helperStep(f: HelperFacts, asked: Boolean = false): HelperStep = when {
    f.sdk < MIN_SDK -> HelperStep.UNSUPPORTED
    !f.paired -> HelperStep.NOT_SET_UP
    f.alive -> HelperStep.NOTHING
    !f.canToggle -> HelperStep.NEEDS_SETUP_AGAIN
    !f.wifi -> HelperStep.NO_WIFI
    !asked && f.msSinceAttempt < HELPER_RETRY_MS -> HelperStep.TOO_SOON
    else -> HelperStep.START
}

fun helperRenewDue(msSinceConnect: Long): Boolean = msSinceConnect >= HELPER_RENEW_MS

/** Wireless debugging is only on for the moment Relay needs it, unless the user had it on. */
fun turnWirelessOffAfter(wasOn: Boolean): Boolean = !wasOn

private fun shellQuote(s: String) = "'" + s.replace("'", "'\\''") + "'"

/**
 * The shell command that starts the helper. The secret goes in the environment, which only this
 * user can read, and never on a command line, which shows in a process list.
 */
fun helperStartCommand(secretHex: String, apk: String): String {
    val path = shellQuote(apk)
    return "${HelperProtocol.SECRET_ENV}=${shellQuote(secretHex)} CLASSPATH=$path " +
        "nohup app_process / dev.phonebridge.helper.Main $path >/dev/null 2>&1 &"
}

/** The digits Android shows, tolerant of spaces. Null when it can't be one. */
fun pairingCode(text: String): String? {
    val digits = text.filterNot { it.isWhitespace() }
    return digits.takeIf { it.length == CODE_DIGITS && it.all { c -> c in '0'..'9' } }
}

/** Settings' words for the state. Only a helper that just answered may read "On". */
fun helperRowText(step: HelperStep): String = when (step) {
    HelperStep.NOTHING -> "On. No prompt, the phone stays silent, and its volume bar sets the level"
    HelperStep.START -> "Starting. Until then, sharing uses Android's capture"
    HelperStep.TOO_SOON -> "Couldn't start just now. It tries again by itself, and when a PC asks"
    HelperStep.NO_WIFI -> "Waiting for Wi-Fi to start. Until then, sharing uses Android's capture"
    HelperStep.NEEDS_SETUP_AGAIN -> "Needs setting up again. Tap to pair once more"
    HelperStep.NOT_SET_UP -> "Off. Tap for one-time setup: no prompt, a silent phone, and its real volume bar"
    HelperStep.UNSUPPORTED -> "Needs Android 12 or later"
}
