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
    ) + (
        // With no PC the hero card already has the Pair button: the same action is not offered twice on Home.
        if (paired) listOf(QuickTile(Quick.Pair, "Pair another PC", false, running)) else emptyList()
    )
}

/** Why a dimmed tile does nothing, for the message when it is tapped. */
fun tileDisabledReason(running: Boolean, paired: Boolean): String =
    if (!running) "Turn on $APP_NAME first" else if (!paired) "Pair a PC first" else ""

/** The PC name on Home's link line; [dim] when it is the remembered name (Relay off or starting), not a live one. */
data class HeroPc(val name: String?, val dim: Boolean)

fun heroPc(linked: String?, stored: String?, running: Boolean, ready: Boolean): HeroPc = when {
    linked != null -> HeroPc(linked, false)
    stored != null && !(running && ready) -> HeroPc(stored, true)
    else -> HeroPc(null, false)
}
