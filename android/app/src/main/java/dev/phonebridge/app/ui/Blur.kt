package dev.phonebridge.app.ui

/** How a glass panel is drawn: real backdrop blur (Haze), a tinted scrim, or an opaque panel. */
enum class BlurMode { Haze, Scrim, Opaque }

/**
 * Blur needs Android 12 (API 31) and not lite mode. Reduced transparency and high contrast always draw opaque
 * panels, on every API level.
 */
fun blurMode(sdk: Int, modes: Modes): BlurMode = when {
    modes.reducedTransparency || modes.highContrast -> BlurMode.Opaque
    modes.lite || sdk < 31 -> BlurMode.Scrim
    else -> BlurMode.Haze
}

/**
 * Watches the frame times while a blurred surface is on screen. It trips once more than `overRatio` of the last
 * `window` frames took longer than 1.5 frame intervals; one slow frame never trips it. When it trips, the app
 * drops to lite mode on its own.
 */
class FrameBudget(
    private val intervalNs: Long,
    private val window: Int = 120,
    private val overRatio: Float = 0.25f,
) {
    private val recent = BooleanArray(window)
    private var next = 0
    private var filled = 0
    private var slow = 0

    /** Records one frame; true when the budget is over. */
    fun onFrame(frameNs: Long): Boolean {
        val isSlow = frameNs > intervalNs * 3 / 2
        if (filled == window) {
            if (recent[next]) slow--
        } else {
            filled++
        }
        recent[next] = isSlow
        if (isSlow) slow++
        next = (next + 1) % window
        return filled == window && slow > window * overRatio
    }
}
