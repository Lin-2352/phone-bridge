package dev.phonebridge.app.ui.home

/** One problem the Home "Needs attention" card lists, each with its own fix. */
enum class Need { NotificationAccess, PhotosPermission, ControlOff, PcForgotPhone }

/**
 * What the "Needs attention" card lists, in its fixed priority order (empty hides the card).
 * [notifyAccess] already means "granted, or not needed" (notifications to the PC are off).
 * Photos matter only while screenshots are shared; Control only while the PC is expected to use it.
 */
fun needs(notifyAccess: Boolean, screenshotsOn: Boolean, photosGranted: Boolean, controlOn: Boolean, controlWaiting: Boolean, pcForgot: Boolean): List<Need> = buildList {
    if (!notifyAccess) add(Need.NotificationAccess)
    if (screenshotsOn && !photosGranted) add(Need.PhotosPermission)
    if (controlWaiting && !controlOn) add(Need.ControlOff)
    if (pcForgot) add(Need.PcForgotPhone)
}

/**
 * The hero card's status line, worded as on the PC: "Connected" only while a session is live,
 * else "Last seen 5 min ago", or "Not seen yet" when the PC never reached this phone ([lastSeenMs] 0).
 */
fun pcStatus(live: Boolean, lastSeenMs: Long, nowMs: Long): String = when {
    live -> "Connected"
    lastSeenMs <= 0L -> "Not seen yet"
    else -> "Last seen ${ago(lastSeenMs, nowMs)}"
}

private fun ago(atMs: Long, nowMs: Long): String {
    val d = (nowMs - atMs).coerceAtLeast(0L) / 1000
    return when {
        d < 60 -> "just now"
        d < 3600 -> "${d / 60} min ago"
        d < 86_400 -> "${d / 3600} h ago"
        d < 2 * 86_400 -> "yesterday"
        else -> "${d / 86_400} days ago"
    }
}
