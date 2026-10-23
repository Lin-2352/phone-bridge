package dev.phonebridge.app

/** Title of the "a PC asks" notification. [what]: "use your camera", "share your screen". */
fun askTitle(what: String, peer: String, locked: Boolean): String =
    if (locked) "Unlock to $what for $peer" else "$peer wants to $what"

fun askBody(locked: Boolean): String =
    if (locked) "Then tap here to allow it" else "Tap here to answer"
