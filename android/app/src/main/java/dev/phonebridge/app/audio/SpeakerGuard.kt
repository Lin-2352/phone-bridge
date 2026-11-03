package dev.phonebridge.app.audio

/** What the owner of a muted stream should do after a frame. */
enum class SpeakerAction { NONE, UNMUTE_TO_CHECK, MUTE_AGAIN, GIVE_UP }

/**
 * Finds out whether muting the phone's media stream also silences the capture, as it may on some
 * phones: a toggle that went on muting there would send nothing to the PC. Silence alone proves
 * nothing, since a video can be quiet. So after [silentLimitMs] of it while sound should be playing,
 * the stream is unmuted to check: sound coming back within [checkMs] proves the mute was the cause;
 * still silence means the video really is quiet, so it mutes again. That check is audible, so it
 * happens only until sound has been captured through the mute, once [settleMs] has passed for the
 * mute to reach the capture: from then on the question is answered. Pure; one call per frame.
 */
class SpeakerGuard(
    private val frameMs: Int,
    private val silentLimitMs: Int = 1500,
    private val checkMs: Int = 1500,
    private val settleMs: Int = 1000,
    proven: Boolean = false,
) {
    private var silentMs = 0
    private var mutedMs = 0

    /** Sound was captured while muted: this phone's capture does not die with the mute. */
    var proven: Boolean = proven
        private set
    private var checking = false

    /** Unmuted just to look: the owner must not re-mute in the meantime. */
    val isChecking: Boolean get() = checking

    fun onFrame(silent: Boolean, soundExpected: Boolean): SpeakerAction {
        if (proven) return SpeakerAction.NONE
        if (checking) {
            if (!silent) return SpeakerAction.GIVE_UP
            silentMs += frameMs
            if (silentMs < checkMs) return SpeakerAction.NONE
            checking = false
            silentMs = 0
            return SpeakerAction.MUTE_AGAIN
        }
        if (mutedMs < settleMs) {
            mutedMs += frameMs
        } else if (!silent) {
            proven = true
            return SpeakerAction.NONE
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
