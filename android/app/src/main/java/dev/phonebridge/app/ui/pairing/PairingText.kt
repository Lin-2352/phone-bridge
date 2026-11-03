package dev.phonebridge.app.ui.pairing

import dev.phonebridge.app.APP_NAME

/*
 * The words of the pairing flow, as pure functions. The FFI's reason strings (pb-transport's Display impls)
 * are matched here and never shown: each case says what happened and what to do next.
 */

private const val SCAN_AGAIN = "scan again"

private const val EXPIRED = "That code has expired. On your PC choose Pair new device to get a new code, then scan it."
private const val REJECTED = "Your PC did not accept that code. On your PC choose Pair new device to show a new one, then scan it."
private const val NOT_RELAY = "That is not a $APP_NAME pairing code. Scan the QR code your PC shows under Pair new device."
private const val UNREACHABLE = "Could not reach your PC. Check that it is on the same Wi-Fi and that $APP_NAME is open there, then scan again."
private const val UNKNOWN = "Pairing did not work. On your PC choose Pair new device to show a new code, then $SCAN_AGAIN."

/** Why the first step (joining with the scanned code) failed, in a sentence with a next step. */
fun joinErrorText(reason: String?): String {
    val r = reason?.lowercase() ?: return UNKNOWN
    return when {
        "not a relay pairing code" in r || "malformed" in r -> NOT_RELAY
        "wrong pairing code" in r || "too many wrong attempts" in r -> REJECTED
        "expired" in r || "no pairing code is active" in r || "already used" in r -> EXPIRED
        "timed out" in r || "transport error" in r || "connection closed" in r -> UNREACHABLE
        else -> UNKNOWN
    }
}

/** Why the last step failed after the user compared the codes; [accepted] is what they chose. */
fun confirmErrorText(accepted: Boolean, reason: String?): String {
    if (!accepted) return "Pairing cancelled. Nothing was paired."
    val r = reason?.lowercase().orEmpty()
    return when {
        "no pairing in progress" in r -> "Pairing was cancelled before it finished. Scan the code again."
        "declined" in r || "refused" in r -> "Your PC did not confirm. Check that its pairing window is still open, then scan again."
        "timed out" in r || "connection closed" in r || "transport error" in r -> "Lost contact with your PC before pairing finished. Check the Wi-Fi and scan again."
        else -> "Pairing was not completed. On your PC choose Pair new device, then scan again."
    }
}

/** The progress line while the PC is contacted; says so honestly once it is slow. */
fun pairingProgressText(elapsedMs: Long): String =
    if (elapsedMs < 10_000) "Contacting your PC\u2026"
    else "Still trying. Your PC may be asleep or on another network. You can Cancel."

private val HEX = Regex("[0-9a-fA-F]+")

/** Shape check for a pasted code `pb1:<64 hex>:<32 hex>:<addresses>`, so an obvious mistake is caught before dialling. */
fun looksLikePairingCode(text: String): Boolean {
    val rest = text.trim().removePrefix("pb1:").takeIf { text.trim().startsWith("pb1:") } ?: return false
    val parts = rest.split(':', limit = 3)
    if (parts.size < 3) return false
    return parts[0].length == 64 && parts[0].matches(HEX) && parts[1].length == 32 && parts[1].matches(HEX)
}

/**
 * Tells the scanner when to say "That is not a Relay code": a QR that is not ours must stay in view for
 * [afterMs]; the hint goes away [goneMs] after it left the view.
 */
class NotRelayTracker(private val afterMs: Long = 1_000, private val goneMs: Long = 1_500) {
    private var since = -1L
    private var last = -1L

    /** [text] is the foreign code in view, or null when nothing decoded; returns whether the hint shows. */
    fun onDecode(text: String?, nowMs: Long): Boolean {
        if (text != null) {
            if (since < 0) since = nowMs
            last = nowMs
        } else if (since >= 0 && nowMs - last > goneMs) {
            since = -1
        }
        return since >= 0 && (text != null || nowMs - last <= goneMs) && nowMs - since >= afterMs
    }
}

/** What to offer after the camera was refused: ask again, or Android's settings page when it will not ask again. */
enum class DenialAction { TryAgain, OpenSettings }

fun denialAction(shouldShowRationale: Boolean) = if (shouldShowRationale) DenialAction.TryAgain else DenialAction.OpenSettings
