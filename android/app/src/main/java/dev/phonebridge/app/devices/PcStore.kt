package dev.phonebridge.app.devices

import uniffi.pb_ffi.PbPeer

/** An answer to a request from one PC: ask each time, always allow, or never ask (refuse without a prompt). */
enum class Allow { Ask, Always, Never }

/**
 * Everything the phone remembers about one paired PC. Keys are `<peerId>/<key>` in one preferences file, so a PC
 * removed from the phone takes its settings with it and a new PC starts from [freshDefaults].
 */
data class PcSettings(
    val nickname: String?,
    val control: Allow,
    val screen: Allow,
    val camera: Allow,
    val audio: Allow,
    val clipboardSync: Boolean,
    val notifications: Boolean,
    val autoAccept: Boolean,
    val lastSeenMs: Long,
)

/** A new PC: ask for everything, share nothing automatically. */
fun freshDefaults(): PcSettings = PcSettings(
    nickname = null,
    control = Allow.Ask,
    screen = Allow.Ask,
    camera = Allow.Ask,
    audio = Allow.Ask,
    clipboardSync = false,
    notifications = false,
    autoAccept = false,
    lastSeenMs = 0L,
)

private fun allow(v: Any?): Allow = when (v) {
    "always" -> Allow.Always
    "never" -> Allow.Never
    else -> Allow.Ask
}

/** The stored value of an [Allow], as written to preferences. */
fun Allow.stored(): String = name.lowercase()

/** One PC's settings from the flat preferences map; a missing or unknown value reads as the fresh default. */
fun pcSettings(all: Map<String, Any?>, id: String): PcSettings {
    val d = freshDefaults()
    fun key(k: String) = "$id/$k"
    return PcSettings(
        nickname = (all[key("nickname")] as? String)?.takeIf { it.isNotBlank() },
        control = allow(all[key("control")]),
        screen = allow(all[key("screen")]),
        camera = allow(all[key("camera")]),
        audio = allow(all[key("audio")]),
        clipboardSync = all[key("clipboard_sync")] as? Boolean ?: d.clipboardSync,
        notifications = all[key("notifications")] as? Boolean ?: d.notifications,
        autoAccept = all[key("auto_accept")] as? Boolean ?: d.autoAccept,
        lastSeenMs = all[key("last_seen_ms")] as? Long ?: d.lastSeenMs,
    )
}

/** Preference keys of the form `<peer id>/...` whose peer is no longer paired: they go when the PC is removed. */
fun keysOfUnpaired(allKeys: Collection<String>, pairedIds: Collection<String>): List<String> {
    val id = Regex("^[0-9a-fA-F]{64}/")
    return allKeys.filter { k -> id.containsMatchIn(k) && k.substringBefore('/') !in pairedIds }
}

/** The default PC after the paired set changed without a removal event: kept while paired, else the first. */
fun defaultIfStillPaired(current: String?, pairedIds: List<String>): String? =
    if (current != null && current in pairedIds) current else pairedIds.firstOrNull()

/** The longest nickname kept. */
const val MAX_NICKNAME = 64

/** A nickname is a local label: control characters are dropped, it is trimmed and cut to [MAX_NICKNAME]. */
fun cleanNickname(raw: String?): String? =
    raw?.filter { !it.isISOControl() }?.trim()?.take(MAX_NICKNAME)?.takeIf { it.isNotEmpty() }

/** What to show for a PC: its nickname if it has one, otherwise the name it announced. */
fun displayName(nickname: String?, name: String): String = cleanNickname(nickname) ?: name

/**
 * A label for each PC, by id. Two PCs with the same display name get the first four characters of their
 * fingerprint added, so they can be told apart; a nickname that breaks the tie needs no suffix.
 */
fun labels(peers: List<PbPeer>, nick: Map<String, String?>): Map<String, String> {
    val base = peers.associate { it.id to displayName(nick[it.id], it.name) }
    val repeated = base.values.groupingBy { it }.eachCount().filterValues { it > 1 }.keys
    return peers.associate { p ->
        val b = base.getValue(p.id)
        p.id to if (b in repeated) "$b (${p.fingerprint.take(4)})" else b
    }
}

/**
 * The default PC after a removal: the current one stays unless it is the one removed; then the first remaining PC
 * by name ignoring case, or none when no PC is left.
 */
fun defaultAfterRemoval(remaining: List<PbPeer>, removed: String, current: String?): String? {
    if (current != null && current != removed && remaining.any { it.id == current }) return current
    return remaining.minByOrNull { it.name.lowercase() }?.id
}

/** The status of one PC in a list. The last-seen time lives on the Home card only. */
fun pcStatus(live: Boolean, paired: Boolean): String = when {
    !paired -> "Not paired"
    live -> "Connected"
    else -> "Not connected"
}

/**
 * Gives every paired PC today's global switches, once (marked `pcs_migrated`). Always stays in the allow sets, so
 * a PC already in one keeps it; the audio, clipboard, notification and automatic-accept switches become each PC's
 * own. Old global keys are kept for one release (a downgrade still finds them). A PC paired later finds no
 * entries and starts from [freshDefaults]. Running it twice changes nothing.
 */
fun migrateToPerPc(all: Map<String, Any?>, pairedIds: List<String>): Map<String, Any?> {
    if (all["pcs_migrated"] == true) return all
    val out = all.toMutableMap()
    val audioOn = all["audio_skip_confirm_phone_to_pc"] == true
    val audio = (all["allow_audio_pcs"] as? Set<*>)?.filterIsInstance<String>()?.toSet().orEmpty()
    if (audioOn) out["allow_audio_pcs"] = audio + pairedIds
    for (id in pairedIds) {
        if (all["clipboard_sync"] == true) out["$id/clipboard_sync"] = true
        // These two are on unless the person turned them off: an install that never chose keeps what it had.
        if (all["notify_to_pc"] != false) out["$id/notifications"] = true
        if (all["auto_accept"] != false) out["$id/auto_accept"] = true
    }
    pairedIds.firstOrNull()?.let { first ->
        (all["last_seen_ms"] as? Long)?.let { out["$first/last_seen_ms"] = it }
        if (all["default_pc"] == null) out["default_pc"] = first
    }
    out["pcs_migrated"] = true
    return out
}

/** Where a send goes: to one PC, to a choice among several, or nowhere because none is paired. */
sealed interface SendTarget {
    data class One(val id: String) : SendTarget

    /** Several PCs: ids ordered default first, then by label ignoring case. */
    data class Ask(val ids: List<String>) : SendTarget

    object None : SendTarget
}

/**
 * Picks the target of a send. An explicit id that is still paired wins; one PC needs no question; several ask with
 * the default first; an explicit id that is no longer paired is ignored.
 */
fun sendTarget(peers: List<PbPeer>, explicit: String?, defaultId: String?): SendTarget {
    if (explicit != null && peers.any { it.id == explicit }) return SendTarget.One(explicit)
    return when (peers.size) {
        0 -> SendTarget.None
        1 -> SendTarget.One(peers[0].id)
        else -> SendTarget.Ask(
            peers.sortedWith(compareBy({ it.id != defaultId }, { it.name.lowercase() })).map { it.id },
        )
    }
}

/** The tile's second line: where a tap sends. */
fun tileSubtitle(defaultLabel: String?): String = if (defaultLabel.isNullOrBlank()) "To your PC" else "To $defaultLabel"

/** The widget's status line: the default PC's label, or the plain state when none is paired. */
fun widgetLabel(defaultLabel: String?, fallback: String): String = defaultLabel?.takeIf { it.isNotBlank() } ?: fallback

/** True when a tap cannot pick a PC by itself: several are paired and none is the saved default. */
fun needsPicker(peers: List<PbPeer>, defaultId: String?): Boolean =
    peers.size > 1 && peers.none { it.id == defaultId }
