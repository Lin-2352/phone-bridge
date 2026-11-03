package dev.phonebridge.app

/** Title of the "a PC asks" notification. [what]: "use your camera", "share your screen". */
fun askTitle(what: String, peer: String, locked: Boolean): String =
    if (locked) "Unlock to $what for $peer" else "$peer wants to $what"

fun askBody(locked: Boolean): String =
    if (locked) "Then tap here to allow it" else "Tap here to answer"

/**
 * Starting phone audio from the PC with nobody touching the phone needs two things only the user
 * can allow: opening a window from the background, and Android's capture prompt auto-approved.
 */
fun handsFreeAudioText(overDisplay: Boolean, captureAllowed: Boolean): String = when {
    overDisplay && captureAllowed -> "On. The PC can start phone audio with nothing to tap"
    overDisplay -> "The window opens by itself, and Android's capture prompt still needs a tap"
    captureAllowed -> "Tap to allow Display over other apps. Until then, the request waits in a notification for 30 s"
    else -> "Tap to allow Display over other apps. Until then, the request waits in a notification for 30 s, and Android's capture prompt needs a tap"
}
