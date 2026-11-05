package dev.phonebridge.app

/**
 * What to update when this phone and [pc] can't talk, from a failure's message (the Rust transport
 * error's words, pinned by its test `version_texts_name_the_device_to_update`). Null for any other
 * failure, so the caller keeps its own text.
 */
fun outdatedText(reason: String?, pc: String): String? {
    val r = reason ?: return null
    return when {
        "update Relay there" in r -> "$pc runs an older version of $APP_NAME. Update $APP_NAME on $pc"
        "update Relay here" in r -> "Update $APP_NAME on this phone: $pc needs a newer version"
        "update both to the latest version" in r -> "$pc runs a different version of $APP_NAME. Update $APP_NAME on both devices"
        else -> null
    }
}
