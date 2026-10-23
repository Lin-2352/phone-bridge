package dev.phonebridge.app

/** What a tap on the Phone Bridge tile does. */
enum class TileClick { StartViaActivity, StartDirect, Stop }

/**
 * Unlocked, turning on never relies on a background service start: some
 * systems (realme) refuse it silently and nothing runs, so the tile goes
 * through an invisible activity, which counts as foreground and may start
 * it. On the lock screen the tap must not ask for a fingerprint first, so
 * it starts the service directly.
 */
fun tileClick(shownOn: Boolean, locked: Boolean): TileClick = when {
    shownOn -> TileClick.Stop
    locked -> TileClick.StartDirect
    else -> TileClick.StartViaActivity
}

/** The tile's second line, from what is really running. */
fun tileSubtitle(wished: Boolean, running: Boolean, starting: Boolean, linked: String?): String = when {
    running -> linked ?: "On"
    starting -> "Starting\u2026"
    // Wanted on, but Android killed it: say so; a tap starts it again.
    wished -> "Stopped. Tap to start"
    else -> "Off"
}
