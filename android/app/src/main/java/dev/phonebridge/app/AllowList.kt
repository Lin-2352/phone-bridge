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

/** The id of the one peer called [name]; empty when none or several have it (never auto-allowed then). */
fun uniquePeerId(peers: List<Pair<String, String>>, name: String): String =
    peers.filter { it.first == name }.singleOrNull()?.second.orEmpty()

/** The list without PCs that are no longer paired (a removed PC must not keep "Always allow"). */
fun pruneAllowed(allowed: Set<String>, pairedIds: List<String>): Set<String> =
    allowed.filter { it in pairedIds }.toSet()
