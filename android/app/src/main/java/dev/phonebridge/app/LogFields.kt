package dev.phonebridge.app

import java.io.File
import java.security.MessageDigest

/** Pure helpers for the day-by-day log: no Android, no FFI, so they are unit-tested. */
object LogFields {
    /** One flat JSON object from [fields]: strings escaped, numbers and booleans bare, nulls skipped. */
    fun json(vararg fields: Pair<String, Any?>): String {
        val sb = StringBuilder("{")
        var first = true
        for ((k, v) in fields) {
            if (v == null) continue
            if (!first) sb.append(',')
            first = false
            quote(sb, k)
            sb.append(':')
            when (v) {
                is Boolean, is Int, is Long -> sb.append(v.toString())
                is Float -> sb.append(if (v.isFinite()) v.toString() else "null")
                is Double -> sb.append(if (v.isFinite()) v.toString() else "null")
                is Number -> sb.append(v.toString())
                else -> quote(sb, v.toString())
            }
        }
        return sb.append('}').toString()
    }

    private fun quote(sb: StringBuilder, s: String) {
        sb.append('"')
        for (c in s) {
            when {
                c == '"' -> sb.append("\\\"")
                c == '\\' -> sb.append("\\\\")
                c == '\n' -> sb.append("\\n")
                c == '\r' -> sb.append("\\r")
                c == '\t' -> sb.append("\\t")
                c < ' ' -> sb.append("\\u%04x".format(c.code))
                else -> sb.append(c)
            }
        }
        sb.append('"')
    }

    /** First 8 hex characters of SHA-256 of [bssid] (lower-cased); null when absent or Android's redacted placeholder. */
    fun hashBssid(bssid: String?): String? {
        val b = bssid?.trim()?.lowercase() ?: return null
        if (b.isEmpty() || b == "02:00:00:00:00:00" || b == "<unknown bssid>") return null
        val d = MessageDigest.getInstance("SHA-256").digest(b.toByteArray())
        return d.take(4).joinToString("") { "%02x".format(it) }
    }

    /** "2.4 GHz", "5 GHz", "6 GHz" for a Wi-Fi centre frequency in MHz, or null. */
    fun band(mhz: Int): String? = when (mhz) {
        in 2400..2500 -> "2.4 GHz"
        in 4900..5900 -> "5 GHz"
        in 5925..7125 -> "6 GHz"
        else -> null
    }

    /** A file name reduced to its extension (at most 5 letters or digits): never the name itself. */
    fun extOnly(name: String): String {
        val e = name.substringAfterLast('.', "").lowercase()
        return if (e.isNotEmpty() && e.length <= 5 && e.all { it.isLetterOrDigit() }) ".$e" else ""
    }

    private val DAY = Regex("""\d{4}-\d{2}-\d{2}""")

    /** Files to export: every regular file inside a `yyyy-MM-dd` folder of [root], as "day/file" entries, oldest day first. */
    fun exportFiles(root: File): List<Pair<String, File>> =
        root.listFiles { f -> f.isDirectory && DAY.matches(f.name) }.orEmpty().sortedBy { it.name }.flatMap { day ->
            day.listFiles { f -> f.isFile }.orEmpty().sortedBy { it.name }.map { "${day.name}/${it.name}" to it }
        }

    /** MB with one decimal, for the Settings row. */
    fun mb(bytes: Long): String = "%.1f MB".format(java.util.Locale.US, bytes / 1_048_576.0)
}
