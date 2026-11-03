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
