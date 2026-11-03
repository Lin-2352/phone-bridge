package dev.phonebridge.app.audio

/** True when every 16-bit LE sample in the first [len] bytes is within the noise floor. */
fun isSilentPcm16(buf: ByteArray, len: Int, floor: Int = 8): Boolean {
    var i = 0
    val end = len - (len and 1)
    while (i < end) {
        val s = (buf[i].toInt() and 0xff) or (buf[i + 1].toInt() shl 8) // sign-extended
        if (s > floor || s < -floor) return false
        i += 2
    }
    return true
}

/**
 * Detects an app that blocks playback capture: Android then delivers silent frames while the
 * app plays sound. Pure and allocation-free; the caller says per frame whether sound is expected
 * (for example AudioManager.isMusicActive).
 */
class SilenceDetector(private val frameMs: Int = 20, private val thresholdMs: Int = 2000) {
    private var silentMs = 0

    var blocked = false
        private set

    /** Feeds one frame; returns true once silence while expecting sound has lasted [thresholdMs]. */
    fun onFrame(buf: ByteArray, len: Int, expectSound: Boolean): Boolean {
        if (!expectSound || !isSilentPcm16(buf, len)) {
            silentMs = 0
            return false
        }
        silentMs += frameMs
        if (silentMs >= thresholdMs) blocked = true
        return blocked
    }

    fun reset() {
        silentMs = 0
        blocked = false
    }
}
