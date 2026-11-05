package dev.phonebridge.app

/** The settings file's format version; a file with no version key is version 0. */
const val PREFS_VERSION = 1
const val PREFS_VERSION_KEY = "prefs_version"

/** 0 for settings from before versions, the current version for a fresh install (nothing stored yet). */
fun prefsVersion(all: Map<String, Any?>): Int =
    (all[PREFS_VERSION_KEY] as? Int) ?: if (all.isEmpty()) PREFS_VERSION else 0

/**
 * Brings stored settings from version [from] to [PREFS_VERSION], one step at a time, on a copy
 * (string sets are copied too). A newer version is returned as it is.
 *
 * Step 0 to 1 removes leftovers of older builds: the unkeyed clipboard hash `last_clip` and the
 * global `allow_camera`. The global `allow_control` and `allow_screen` are NOT folded here: Settings
 * writes them on purpose while the node is off ("pending"), so [Prefs.migrateAllowances] folds them
 * into the paired PCs at every start.
 */
fun migratePrefs(from: Int, all: Map<String, Any?>): Pair<Int, Map<String, Any?>> {
    var out: Map<String, Any?> = all.mapValues { (_, v) -> if (v is Set<*>) v.toSet() else v }
    var v = from
    while (v < PREFS_VERSION) {
        if (v == 0) out = out - "last_clip" - "allow_camera"
        v++
    }
    return maxOf(v, from) to out
}

/** What to write to bring [before] to [after]. */
data class PrefsEdit(val put: Map<String, Any>, val remove: Set<String>)

fun prefsDiff(before: Map<String, Any?>, after: Map<String, Any?>): PrefsEdit = PrefsEdit(
    put = after.filter { (k, v) -> v != null && before[k] != v }.mapValues { it.value!! },
    remove = before.keys.filter { it !in after }.toSet(),
)
