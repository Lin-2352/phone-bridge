package dev.phonebridge.app

/** One tile on the home screen; [on] draws it in the accent colour, [enabled] is false while no PC is linked. */
data class HomeTile(val group: Group, val label: String, val on: Boolean, val enabled: Boolean)

/**
 * The home tiles in display order. A live Camera or Screen tile stays enabled without a connection so it can
 * still be stopped; Clipboard is a local setting, so it is always enabled. Send file is never on.
 */
fun homeTiles(connected: Boolean, cameraOn: Boolean, screenOn: Boolean, clipboardOn: Boolean = false): List<HomeTile> = listOf(
    HomeTile(Group.Camera, "Camera", cameraOn, connected || cameraOn),
    HomeTile(Group.Screen, "Screen", screenOn, connected || screenOn),
    HomeTile(Group.Settings, "Clipboard", clipboardOn, true),
    HomeTile(Group.Files, "Send file", false, connected),
)
