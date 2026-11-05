package dev.phonebridge.app

/** EXTRA_TEXT as plain text: styled (Spanned) text is a CharSequence, not a String. Blank is nothing. */
fun sharedText(extra: CharSequence?): String? = extra?.toString()?.takeIf { it.isNotBlank() }

/** The share toast, promising only what will really be sent. */
fun shareToast(peerName: String?, files: Int, hasText: Boolean, dropped: Int): String {
    val to = peerName ?: "your PC"
    if (files == 0 && !hasText) return "Nothing to send"
    val skipped = when (dropped) {
        0 -> ""
        1 -> " 1 file could not be read"
        else -> " $dropped files could not be read"
    }
    return when {
        dropped > 0 && files > 1 -> "Sending $files files to $to.$skipped"
        dropped > 0 -> "Sending to $to.$skipped"
        else -> "Sending to $to"
    }
}

/** What shared text becomes on the PC: a link it offers to open, or text it offers to copy. */
sealed interface SharedTextKind {
    data class Link(val url: String) : SharedTextKind
    data class Text(val text: String) : SharedTextKind
}

private val WEB_LINK = Regex("""(?i)\bhttps?://[^\s<>"]+""")

/** Words next to a link up to this length (a page title) go with it; longer text is a message. */
private const val LINK_CONTEXT_CHARS = 120

/**
 * A short text holding a web link is that link (browsers share "Title https://…"). The scheme is
 * lowercased: Android matches schemes case-sensitively, so "HTTPS://" would find no handler.
 */
fun classifySharedText(text: String): SharedTextKind {
    val m = WEB_LINK.find(text) ?: return SharedTextKind.Text(text)
    if (text.removeRange(m.range).trim().length > LINK_CONTEXT_CHARS) return SharedTextKind.Text(text)
    val url = m.value.trimEnd('.', ',', ';', ':', '!', '?')
    val i = url.indexOf("://")
    return SharedTextKind.Link(url.substring(0, i).lowercase() + url.substring(i))
}

/** The text part of a share. It goes along with any files (it used to be dropped then). */
fun textToSend(text: String?): SharedTextKind? = text?.let(::classifySharedText)

/** The confirm sheet's preview: how many files and which, then the start of the text. */
fun sharePreview(text: String?, names: List<String>): String {
    val parts = mutableListOf<String>()
    if (names.isNotEmpty()) {
        val more = names.size - 3
        parts += "${names.size} ${if (names.size == 1) "file" else "files"}: ${names.take(3).joinToString(", ")}" +
            if (more > 0) " and $more more" else ""
    }
    if (text != null) parts += if (text.length > 200) text.take(200) + "…" else text
    return parts.joinToString("\n")
}

/**
 * Whether a content URI is the save folder [tree] or a document inside it. Compared by authority
 * (without a "0@" user prefix) and decoded document ID, so spellings such as %3a and %3A match,
 * as they do for Android's own grant check.
 */
fun isUnderTree(uri: String, tree: String): Boolean {
    val (ua, uid) = authorityAndDocId(uri) ?: return false
    val (ta, tid) = authorityAndDocId(tree) ?: return false
    return ua == ta && (uid == tid || uid.startsWith("$tid/"))
}

private fun authorityAndDocId(s: String): Pair<String, String>? {
    if (!s.startsWith("content://", ignoreCase = true)) return null
    val rest = s.substring("content://".length)
    val slash = rest.indexOf('/').takeIf { it >= 0 } ?: return null
    val authority = rest.substring(0, slash).substringAfterLast('@').lowercase()
    val segs = rest.substring(slash + 1).substringBefore('?').split('/')
    // ".../tree/<id>[/document/<id>]" or ".../document/<id>": the last ID names the item.
    val i = segs.lastIndexOf("document").takeIf { it >= 0 } ?: segs.lastIndexOf("tree")
    if (i < 0 || i + 1 >= segs.size) return null
    return authority to percentDecode(segs[i + 1])
}

private fun percentDecode(s: String): String {
    val out = java.io.ByteArrayOutputStream()
    var i = 0
    while (i < s.length) {
        if (s[i] == '%' && i + 2 < s.length) {
            val b = s.substring(i + 1, i + 3).toIntOrNull(16)
            if (b != null) {
                out.write(b)
                i += 3
                continue
            }
        }
        out.write(s[i].toString().toByteArray(Charsets.UTF_8))
        i++
    }
    return String(out.toByteArray(), Charsets.UTF_8)
}
