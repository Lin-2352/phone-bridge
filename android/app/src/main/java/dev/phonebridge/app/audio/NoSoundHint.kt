package dev.phonebridge.app.audio

/** True when a 16-bit PCM frame holds nothing above the [floor]. */
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
 * When to say that nothing is reaching the PC while an app seems to be playing. It can't say why: a
 * player with nothing to say (YouTube's home screen keeps one open) looks the same as an app that
 * doesn't allow capture. So this is only ever a line of text, and a share never ends for it: an app
 * that blocks capture silences only its own sound, and other apps still come through. Pure.
 */
class NoSoundHint(private val afterMs: Long) {
    private var quietSince = -1L

    var showing = false
        private set

    /** Returns true when [showing] changed. */
    fun onTick(nowMs: Long, soundCaptured: Boolean, playerActive: Boolean): Boolean {
        if (soundCaptured || !playerActive) {
            quietSince = -1L
            return show(false)
        }
        if (quietSince < 0) quietSince = nowMs
        return show(nowMs - quietSince >= afterMs)
    }

    private fun show(now: Boolean): Boolean {
        val changed = now != showing
        showing = now
        return changed
    }
}
