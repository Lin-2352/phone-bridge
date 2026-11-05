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

/** The asking PC's id from the event, if it is still paired; otherwise empty (never auto-allowed). Names are not used. */
fun askPeerId(eventId: String, pairedIds: List<String>): String = if (eventId in pairedIds) eventId else ""

/** The list without PCs that are no longer paired (a removed PC must not keep "Always allow"). */
fun pruneAllowed(allowed: Set<String>, pairedIds: List<String>): Set<String> =
    allowed.filter { it in pairedIds }.toSet()

/** Moves an old global "Always allow" to the PCs paired now, so the existing PC keeps its behaviour. */
fun migrateGlobalAllow(oldOn: Boolean, current: Set<String>, pairedIds: List<String>): Set<String> =
    if (oldOn) current + pairedIds.filter { it.isNotEmpty() } else current
