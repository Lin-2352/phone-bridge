package dev.phonebridge.app

/** The Home quick actions, in grid order (2 x 3). */
enum class Quick { SendFile, SendClipboard, Screen, Camera, Touchpad, Pair }

/** One quick action; [on] draws it in the accent colour (a live share), [enabled] says whether it can be tapped. */
data class QuickTile(val quick: Quick, val label: String, val on: Boolean, val enabled: Boolean)

/**
 * The quick actions. Sending and the touchpad need a paired PC and a running node; Pair needs the node.
 * Screen and Camera start from the PC, so on the phone they only stop a live share: a live one stays
 * enabled even without a connection so it can still be stopped.
 */
fun quickTiles(running: Boolean, paired: Boolean, cameraOn: Boolean, screenOn: Boolean): List<QuickTile> {
    val linked = running && paired
    return listOf(
        QuickTile(Quick.SendFile, "Send file", false, linked),
        QuickTile(Quick.SendClipboard, "Send clipboard", false, linked),
        QuickTile(Quick.Screen, "Screen", screenOn, linked || screenOn),
        QuickTile(Quick.Camera, "Camera", cameraOn, linked || cameraOn),
        QuickTile(Quick.Touchpad, "Touchpad", false, linked),
        QuickTile(Quick.Pair, "Pair", false, running),
    )
}
