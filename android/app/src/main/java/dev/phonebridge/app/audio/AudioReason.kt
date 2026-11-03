package dev.phonebridge.app.audio

/** Which audio an [dev.phonebridge.app.AudioStreamer] captures. Chosen by the caller, never inferred. */
enum class AudioSource { PLAYBACK, MICROPHONE }

/** Refusal codes; the numbers are the wire `AudioReason` codes (docs/WIRE_PROTOCOL.md). */
enum class AudioReason(val code: Int) {
    DECLINED(1),
    UNAVAILABLE(2),
    BUSY(3),
    BAD_PARAMS(4),
    NO_PERMISSION(5),
    APP_BLOCKS_CAPTURE(6);

    companion object {
        /** Unknown codes read as [DECLINED], like the Rust side. */
        fun fromCode(code: Int): AudioReason = entries.firstOrNull { it.code == code } ?: DECLINED
    }
}

/**
 * Pure pre-flight check. Returns the reason capture cannot start, or null when it can.
 * Playback capture needs Android 10+ and a MediaProjection token; both sources need RECORD_AUDIO.
 */
fun planCapture(
    source: AudioSource,
    sdkInt: Int,
    hasRecordAudio: Boolean,
    hasProjection: Boolean,
    sampleRate: Int,
    channels: Int,
): AudioReason? {
    if ((sampleRate != 44100 && sampleRate != 48000) || (channels != 1 && channels != 2)) {
        return AudioReason.BAD_PARAMS
    }
    if (source == AudioSource.PLAYBACK && (sdkInt < 29 || !hasProjection)) return AudioReason.UNAVAILABLE
    if (!hasRecordAudio) return AudioReason.NO_PERMISSION
    return null
}

/** Maps an exception thrown while creating or starting the recorder to a wire reason. */
fun reasonForFailure(e: Throwable): AudioReason =
    if (e is SecurityException) AudioReason.NO_PERMISSION else AudioReason.UNAVAILABLE
