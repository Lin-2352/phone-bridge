package dev.phonebridge.app.audio

/** What the user (or Android) must still agree to before the phone's audio is shared. */
enum class ConsentStep {
    /** Relay's own confirmation dialog. */
    RELAY_DIALOG,

    /** The runtime RECORD_AUDIO grant (both sources need it). */
    RECORD_AUDIO,

    /** Android's screen-capture dialog (the MediaProjection token for playback capture). */
    PROJECTION,
}

/**
 * The consent steps for sharing this phone's audio, in order.
 *
 * Playback: Relay's dialog (removed only by the "don't ask" setting [skipRelayDialog]), RECORD_AUDIO if
 * missing, then Android's own dialog, which no setting can remove.
 * Microphone: Relay's dialog every time, whatever [skipRelayDialog] says, then RECORD_AUDIO if missing.
 */
fun consentSteps(source: AudioSource, skipRelayDialog: Boolean, hasRecordAudio: Boolean): List<ConsentStep> = buildList {
    if (source == AudioSource.MICROPHONE || !skipRelayDialog) add(ConsentStep.RELAY_DIALOG)
    if (!hasRecordAudio) add(ConsentStep.RECORD_AUDIO)
    if (source == AudioSource.PLAYBACK) add(ConsentStep.PROJECTION)
}

/** [wire] is `AudioStart.direction`. */
enum class AudioDirection(val wire: Int) {
    PHONE_TO_PC(0),
    PC_TO_PHONE(1);

    companion object {
        fun fromWire(n: Int): AudioDirection? = entries.firstOrNull { it.wire == n }
    }
}

/** One audio session per direction at a time. */
class AudioSessions {
    private val active = BooleanArray(AudioDirection.entries.size)

    /** False when [d] already has a session. */
    @Synchronized fun tryBegin(d: AudioDirection): Boolean {
        if (active[d.ordinal]) return false
        active[d.ordinal] = true
        return true
    }

    @Synchronized fun end(d: AudioDirection) {
        active[d.ordinal] = false
    }

    @Synchronized fun isActive(d: AudioDirection): Boolean = active[d.ordinal]
}

/** Why a running session ended, as far as the user is concerned. */
enum class EndCause { PEER_STOPPED, LOCAL_STOP, LINK_LOST, VIOLATION, WRITE_FAILED, SOURCE_ENDED }

/**
 * The cause to act on: what the node [reported] unless our own capture ended first ([override]). A stop the
 * PC made is never rewritten, so it stays silent even if the capture dies with the link at the same time.
 */
fun resolveEnd(reported: EndCause, override: EndCause?): EndCause =
    if (reported == EndCause.PEER_STOPPED) reported else override ?: reported

/** What to tell the user when a session ends; null when they caused it (or the PC did on purpose). */
fun endNotice(cause: EndCause, direction: AudioDirection): String? {
    val what = if (direction == AudioDirection.PHONE_TO_PC) "Sharing audio to the PC" else "Playing PC audio"
    return when (cause) {
        EndCause.LOCAL_STOP, EndCause.PEER_STOPPED -> null
        EndCause.LINK_LOST -> "$what stopped because the connection to the PC was lost. Check that both are on the same network, then ask again."
        EndCause.VIOLATION -> "$what stopped because the PC sent audio Relay could not accept. Update Relay on both devices, then ask again."
        EndCause.WRITE_FAILED -> "$what stopped because sending to the PC failed. Check the connection, then ask again."
        EndCause.SOURCE_ENDED -> "$what stopped because Android ended the capture (the sharing was closed from the status bar, or the phone ran low on resources). Ask again to restart it."
    }
}

/** What to tell the user, and what to do next, when audio could not start. */
fun refusalNotice(reason: AudioReason, source: AudioSource): String = when (reason) {
    AudioReason.DECLINED -> "Audio sharing was declined. Ask again from the PC when you want it."
    AudioReason.UNAVAILABLE ->
        if (source == AudioSource.PLAYBACK) {
            "Relay could not capture this phone's audio. It needs Android 10 or newer and the screen-capture permission; ask again and allow the Android prompt."
        } else {
            "Relay could not open the microphone. Close other apps that use it, then ask again."
        }
    AudioReason.BUSY -> "Audio is already being shared in this direction. Stop it from the notification first, then ask again."
    AudioReason.BAD_PARAMS -> "The PC asked for an audio format Relay does not support. Update Relay on the PC and try again."
    AudioReason.NO_PERMISSION ->
        if (source == AudioSource.MICROPHONE) {
            "Relay has no microphone permission. Open Settings, Apps, Relay, Permissions, allow Microphone, then ask again."
        } else {
            "Relay has no permission to record audio. Open Settings, Apps, Relay, Permissions, allow Microphone, then ask again."
        }
    AudioReason.APP_BLOCKS_CAPTURE -> "The app that is playing sound does not allow capture. Play it in another app and ask again."
}

/** The persistent notification's text while a session runs. */
fun sessionNoticeText(direction: AudioDirection, source: AudioSource): String = when {
    source == AudioSource.MICROPHONE -> "Microphone is being shared"
    direction == AudioDirection.PHONE_TO_PC -> "Phone audio is playing on the PC"
    else -> "PC audio is playing on this phone"
}

/** The line under [sessionNoticeText]. [noSound]: see [NoSoundHint]. */
fun sessionDetail(direction: AudioDirection, peer: String, noSound: Boolean, linked: Boolean = true): String = when {
    // The PC's session is gone while the share runs: it comes back by itself (the PC dials again).
    !linked -> "Reconnecting to $peer…"
    noSound -> "Nothing to send to $peer right now. Some apps don't allow their sound to be shared."
    direction == AudioDirection.PHONE_TO_PC -> "Sent to $peer"
    else -> "From $peer"
}
