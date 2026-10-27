package dev.phonebridge.app

/** A picture received from the PC and kept so it can sit on the clipboard. */
data class CachedClip(val name: String, val modifiedMs: Long)

private const val DAY_MS = 24L * 60 * 60 * 1000

/**
 * Names of cached pictures to delete: all but the newest [keep], and anything
 * older than [maxAgeMs]. The newest picture stays while it is recent, so the one
 * on the clipboard is never removed from under a paste.
 */
fun clipsToDelete(
    files: List<CachedClip>,
    nowMs: Long,
    keep: Int = 5,
    maxAgeMs: Long = DAY_MS,
): List<String> {
    val newestFirst = files.sortedByDescending { it.modifiedMs }
    return newestFirst.filterIndexed { i, f -> i >= keep || nowMs - f.modifiedMs > maxAgeMs }.map { it.name }
}
