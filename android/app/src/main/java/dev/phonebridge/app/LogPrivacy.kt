package dev.phonebridge.app

import java.io.File

/** A logcat line about a notification: the key's hash only, never the posting app's package. */
fun notifyLogLine(event: String, keyHash: String): String = "NOTIFY $event key=$keyHash"

/** Where the logs go ([current], in private storage) and an older folder to delete ([legacy]). */
data class LogDirs(val current: File, val legacy: File?)

/**
 * Logs live in private storage. Older builds wrote them to app-specific external storage, which
 * other apps with legacy storage access could read on Android 10; that folder is deleted.
 */
fun logDirs(filesDir: File, externalLogs: File?): LogDirs {
    val current = File(filesDir, "logs")
    return LogDirs(current, externalLogs?.takeIf { it.absoluteFile != current.absoluteFile })
}
