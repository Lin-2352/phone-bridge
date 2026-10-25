package dev.phonebridge.app

/** One tile on the home screen; [on] draws it in the accent colour, [enabled] is false while no PC is linked. */
data class HomeTile(val group: Group, val label: String, val on: Boolean, val enabled: Boolean)

/** The home tiles in display order. Clipboard and Send file are never "on" here: the screen overrides clipboard from its setting. */
fun homeTiles(connected: Boolean, cameraOn: Boolean, screenOn: Boolean): List<HomeTile> = listOf(
    HomeTile(Group.Camera, "Camera", cameraOn, connected),
    HomeTile(Group.Screen, "Screen", screenOn, connected),
    HomeTile(Group.Settings, "Clipboard", false, connected),
    HomeTile(Group.Files, "Send file", false, connected),
)
