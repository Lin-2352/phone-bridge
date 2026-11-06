package dev.phonebridge.app.ui.home

import dev.phonebridge.app.APP_NAME

/** The confirmation shown before "Remove and pair again" deletes pairings. */
data class ForgetPrompt(val title: String, val body: String)

fun forgetPrompt(names: List<String>): ForgetPrompt {
    val who = names.map { it.ifBlank { "your PC" } }.distinct()
    val list = when (who.size) {
        0 -> "the old pairing"
        1 -> who[0]
        else -> who.dropLast(1).joinToString(", ") + " and " + who.last()
    }
    return ForgetPrompt(
        title = if (who.size > 1) "Remove these PCs?" else "Remove ${who.firstOrNull() ?: "this PC"}?",
        body = "This removes $list from this phone, so it can no longer connect until you pair again. " +
            "Only continue if you recognise the reset. Then scan the QR code on your PC to pair again.",
    )
}

/** The confirmation shown before "Reset and pair again" (M3.7, B5). */
fun identityResetPrompt(): ForgetPrompt = ForgetPrompt(
    title = "Reset and pair again?",
    body = "$APP_NAME can no longer open the keys that identify this phone, usually after the screen lock was changed or the " +
        "phone was restored. Resetting makes new ones. Your PCs will see this phone as a new device: remove it on each PC, " +
        "then pair again by scanning the QR code on your PC. Nothing else on this phone is deleted.",
)
