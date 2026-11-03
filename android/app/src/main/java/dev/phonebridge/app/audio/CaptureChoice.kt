package dev.phonebridge.app.audio

/** Where a share's sound comes from. */
enum class CaptureKind {
    /** Android's capture API: needs its prompt, or the grant that skips it, and leaves the speaker on. */
    ANDROID,

    /** The audio helper: no prompt, sound off the speaker, the phone's real volume bar sets the level. */
    HELPER,
}

/**
 * The helper only serves the phone's own audio while the phone should be silent, since its route
 * takes the sound off the speaker. It is never a stand-in for the microphone.
 */
fun chooseCapture(source: AudioSource, silencePhone: Boolean, helperAlive: Boolean): CaptureKind =
    if (source == AudioSource.PLAYBACK && silencePhone && helperAlive) CaptureKind.HELPER else CaptureKind.ANDROID

/** Nothing to ask and nothing to grant: the share can start with no window at all. */
fun startsWithoutWindow(kind: CaptureKind, skipConfirm: Boolean): Boolean =
    kind == CaptureKind.HELPER && skipConfirm
