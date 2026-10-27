package dev.phonebridge.app

/** What to tell the user when sending a picture to the PC failed; [error] is the node's message, if any. */
fun imageSendNotice(error: String?): String {
    val e = error.orEmpty().lowercase()
    return when {
        "declin" in e -> "Your PC didn't take the picture. Turn on Share images in $APP_NAME on the PC"
        "too large" in e || "10 mb" in e -> "The picture is over 10 MB, so it wasn't sent"
        else -> "Couldn't reach your PC to send the picture"
    }
}

private val PNG_SIGNATURE = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)

/** Whether [b] starts with the PNG signature (anything else is converted before it is sent). */
fun isPngBytes(b: ByteArray): Boolean = b.size >= PNG_SIGNATURE.size && PNG_SIGNATURE.indices.all { b[it] == PNG_SIGNATURE[it] }
