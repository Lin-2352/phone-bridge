package dev.phonebridge.app

/** Whether this PC (by peer id) was given "Always allow". An empty id is never allowed. */
fun allowedFor(peerId: String, allowed: Set<String>): Boolean =
    peerId.isNotEmpty() && peerId in allowed

/** The list with this PC added or removed; the others are untouched. An empty id changes nothing. */
fun withAllowed(allowed: Set<String>, peerId: String, on: Boolean): Set<String> = when {
    peerId.isEmpty() -> allowed
    on -> allowed + peerId
    else -> allowed - peerId
}
