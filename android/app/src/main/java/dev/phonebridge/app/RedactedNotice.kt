package dev.phonebridge.app

import android.content.res.Resources

/** What the PC shows instead of Android's placeholder, so a missing "Copy code" makes sense. */
const val HIDDEN_CODE_TEXT = "Code hidden by Android: open it on the phone"

/**
 * Android 15 and later hide one-time codes from notification listeners like Relay: the text
 * arrives as Android's own placeholder (A2).
 */
fun isRedactedText(text: CharSequence?, placeholders: Set<String>): Boolean =
    text != null && text.toString().trim() in placeholders

/** The notification text for the PC: the placeholder becomes [HIDDEN_CODE_TEXT]. */
fun textForPc(text: String, placeholders: Set<String>): String =
    if (isRedactedText(text, placeholders)) HIDDEN_CODE_TEXT else text

/**
 * Android's placeholder texts, looked up by resource name; a name this Android version doesn't
 * have is skipped. The names are AOSP's (verify on an Android 15+ phone).
 */
fun systemPlaceholders(res: Resources): Set<String> = PLACEHOLDER_NAMES.mapNotNull { name ->
    @Suppress("DiscouragedApi")
    val id = res.getIdentifier(name, "string", "android")
    if (id == 0) null else runCatching { res.getString(id).trim() }.getOrNull()?.takeIf { it.isNotEmpty() }
}.toSet()

private val PLACEHOLDER_NAMES = listOf(
    "redacted_notification_message",
    "redacted_otp_notification_text",
    "redacted_notification_single_line_text",
)
