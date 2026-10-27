package dev.phonebridge.app

/** Largest picture sent to the PC; mirrors the 10 MiB limit on the wire. */
const val MAX_IMAGE_BYTES = 10L * 1024 * 1024

/** How many handled screenshot ids are remembered. */
const val SEEN_LIMIT = 50

/** A screenshot older than this when the gallery changes is not sent. */
const val RECENT_SECONDS = 15L

/** One row of the phone's picture index, as far as the filter needs it. */
data class MediaRow(
    val id: Long,
    val relativePath: String?,
    val displayName: String?,
    val dateAddedSec: Long,
    val isPending: Boolean,
    val sizeBytes: Long,
)

/**
 * Whether [row] is a screenshot that was just taken and has not been handled:
 * finished writing, recent, of a sendable size, and in a Screenshots folder or
 * named Screenshot_*. Other photos in the gallery are never sent.
 */
fun isScreenshot(row: MediaRow, nowSec: Long, seen: Set<Long>): Boolean {
    if (row.id in seen || row.isPending) return false
    if (row.sizeBytes <= 0 || row.sizeBytes > MAX_IMAGE_BYTES) return false
    if (nowSec - row.dateAddedSec > RECENT_SECONDS) return false
    val inFolder = row.relativePath?.contains("screenshots", ignoreCase = true) == true
    val named = row.displayName?.startsWith("Screenshot", ignoreCase = true) == true
    return inFolder || named
}

/** [seen] plus [id], keeping only the newest [SEEN_LIMIT] ids. */
fun rememberSeen(seen: List<Long>, id: Long): List<Long> =
    if (id in seen) seen else (seen + id).takeLast(SEEN_LIMIT)

/** Whether the home screen should warn that screenshots cannot be sent (switch on, Android access missing). */
fun screenshotAccessMissing(share: Boolean, granted: Boolean): Boolean = share && !granted
