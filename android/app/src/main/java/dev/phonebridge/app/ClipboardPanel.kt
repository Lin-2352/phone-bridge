package dev.phonebridge.app

/** The switch summarizes all PCs; a manual send needs only one enabled PC. */
data class ClipboardPanel(val checked: Boolean, val canSend: Boolean)

fun clipboardPanel(running: Boolean, anyPc: Boolean, allPcs: Boolean) =
    ClipboardPanel(checked = allPcs, canSend = running && anyPc)
