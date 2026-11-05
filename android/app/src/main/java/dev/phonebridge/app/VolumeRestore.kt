package dev.phonebridge.app

/** What to do about the volume a share held (muted or stepped down). */
enum class VolumeAction { RestoreNow, Defer, Nothing }

/** Android 17 (API 37) hardens background audio: volume calls fail silently without a visible window or foreground service. */
private const val AUDIO_HARDENING_SDK = 37

/**
 * Restore at once where Android lets it work; otherwise remember it for the next service start or
 * app open, where it does work (A8).
 */
fun volumeRestoreAction(sdkInt: Int, inForegroundService: Boolean, pending: Boolean): VolumeAction = when {
    !pending -> VolumeAction.Nothing
    inForegroundService || sdkInt < AUDIO_HARDENING_SDK -> VolumeAction.RestoreNow
    else -> VolumeAction.Defer
}
