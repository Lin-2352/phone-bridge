package dev.phonebridge.app.audio

/** What the owner of a muted stream should do after a frame. */
enum class SpeakerAction { NONE, UNMUTE_TO_CHECK, MUTE_AGAIN, GIVE_UP }

/**
 * Finds out whether muting the phone's media stream also silences the capture, as it may on some
 * phones: a toggle that went on muting there would send nothing to the PC. Silence alone proves
 * nothing, since a video can be quiet. So after [silentLimitMs] of it while sound should be playing,
 * the stream is unmuted to check: sound coming back within [checkMs] proves the mute was the cause;
 * still silence means the video really is quiet, so it mutes again. Pure; one call per frame.
 */
class SpeakerGuard(
    private val frameMs: Int,
    private val silentLimitMs: Int = 1500,
    private val checkMs: Int = 1500,
) {
    private var silentMs = 0
    private var checking = false

    /** Unmuted just to look: the owner must not re-mute in the meantime. */
    val isChecking: Boolean get() = checking

    fun onFrame(silent: Boolean, soundExpected: Boolean): SpeakerAction {
        if (checking) {
            if (!silent) return SpeakerAction.GIVE_UP
            silentMs += frameMs
            if (silentMs < checkMs) return SpeakerAction.NONE
            checking = false
            silentMs = 0
            return SpeakerAction.MUTE_AGAIN
        }
        if (!silent || !soundExpected) {
            silentMs = 0
            return SpeakerAction.NONE
        }
        silentMs += frameMs
        if (silentMs < silentLimitMs) return SpeakerAction.NONE
        checking = true
        silentMs = 0
        return SpeakerAction.UNMUTE_TO_CHECK
    }
}
