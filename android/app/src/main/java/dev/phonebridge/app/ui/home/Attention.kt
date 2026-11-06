package dev.phonebridge.app.ui.home

import dev.phonebridge.app.screenshotAccessMissing

/** One problem the Home "Needs attention" card lists, each with its own fix. */
enum class Need { IdentityLost, NotificationAccess, PhotosPermission, ControlOff, PcForgotPhone, AppNotificationsOff, VpnOn }

/**
 * A PC asked to control this phone and was refused because Relay control was not running
 * ([refusedAtMs] from the FFI's `controlRefusedAt()`, 0 for never). It counts as waiting for [windowMs]
 * (the PC user is likely still there), and not at all once control runs. A refusal "in the future"
 * (the clock moved back) does not count, so it cannot stick.
 */
fun controlWaiting(refusedAtMs: Long, nowMs: Long, controlOn: Boolean, windowMs: Long = 120_000): Boolean =
    !controlOn && refusedAtMs > 0 && nowMs - refusedAtMs in 0..windowMs

/**
 * What the "Needs attention" card lists, in its fixed priority order (empty hides the card).
 * [notifyAccess] already means "granted, or not needed" (notifications to the PC are off).
 * Photos matter only while screenshots are shared; Control only while a PC's request waits (off alone is
 * normal: it switches itself off after every session). [appNotificationsOn] is the app's own notifications.
 */
fun needs(notifyAccess: Boolean, screenshotsOn: Boolean, photosGranted: Boolean, controlOn: Boolean, controlWaiting: Boolean, pcForgot: Boolean, appNotificationsOn: Boolean, identityLost: Boolean = false, vpnOn: Boolean = false): List<Need> = buildList {
    // Nothing else works until the identity is reset.
    if (identityLost) add(Need.IdentityLost)
    if (!notifyAccess) add(Need.NotificationAccess)
    if (screenshotAccessMissing(screenshotsOn, photosGranted)) add(Need.PhotosPermission)
    if (controlWaiting && !controlOn) add(Need.ControlOff)
    if (pcForgot) add(Need.PcForgotPhone)
    if (!appNotificationsOn) add(Need.AppNotificationsOff)
    // Last: only a hint. A split-tunnel VPN may still let the LAN through.
    if (vpnOn) add(Need.VpnOn)
}

/**
 * The VPN hint's body, or null when there is nothing to say: no VPN, no PC paired yet, or the link works
 * (a split-tunnel VPN lets the LAN through, and a card that never goes away would only nag).
 */
fun vpnHint(vpnOn: Boolean, paired: Boolean, reachFailed: Boolean): String? =
    if (vpnOn && paired && reachFailed) "A VPN is on. If your PC can't reach this phone, turn the VPN off or allow local network access" else null

/**
 * The hero card's status line, worded as on the PC: "Connected" only while a session is live,
 * else "Last seen 5 min ago". With no contact stamp ([lastSeenMs] 0): "Paired" when a PC is [paired]
 * (an upgrade from a version that kept no stamp: the PC has reached this phone before), else "Not seen yet".
 */
fun pcStatus(live: Boolean, lastSeenMs: Long, nowMs: Long, paired: Boolean): String = when {
    live -> "Connected"
    lastSeenMs <= 0L -> if (paired) "Paired" else "Not seen yet"
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
