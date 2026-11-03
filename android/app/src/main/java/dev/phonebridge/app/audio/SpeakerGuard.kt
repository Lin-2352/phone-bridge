package dev.phonebridge.app.audio

/**
 * Decides whether muting the phone's media stream is compatible with capturing it. On some phones
 * capture comes after the volume stage, so a muted stream captures silence: a toggle that went on
 * muting there would send nothing to the PC. Pure; the caller feeds it one result per frame.
 */
class SpeakerGuard(private val frameMs: Int, private val giveUpAfterMs: Int = 1000) {
    private var silentMs = 0

    /** True once the capture has stayed silent this long while sound should be playing. */
    var captureDied = false
        private set

    fun onFrame(silent: Boolean, soundExpected: Boolean): Boolean {
        if (captureDied) return true
        if (!silent || !soundExpected) {
            silentMs = 0
            return false
        }
        silentMs += frameMs
        if (silentMs >= giveUpAfterMs) captureDied = true
        return captureDied
    }
}
