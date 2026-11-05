package dev.phonebridge.app.audio

/** Relative steps that give the phone its own volume back. Negative [saved]: no share was running. */
fun stepsBack(current: Int, saved: Int): Int = if (saved < 0) 0 else saved - current

/** The PC level a muted share starts at. A saved 0 would be silence with nothing on screen to explain it. */
fun muteShareStartStep(saved: Int): Int = if (saved <= 0) PC_LEVEL_STEPS else clampStep(saved)

/**
 * The PC level a share starts at. [oldIndex]: where a version before this left the volume bar, which
 * set the level then, so nothing jumps in the user's headphones; -1 once it has been taken over.
 */
fun shareStartStep(saved: Int, oldIndex: Int, max: Int): Int =
    if (oldIndex > 0) stepForIndex(oldIndex, max) else muteShareStartStep(saved)

/** The level to keep for the next share. Silence is for now: the next share comes back at one that was heard. */
fun rememberedStep(saved: Int, step: Int): Int = if (step > 0) step else saved

/** Whether a share keeps the phone silent right now. */
fun silenceWanted(silencePhone: Boolean, silenceEarphones: Boolean, earphonesOn: Boolean): Boolean =
    silencePhone && (silenceEarphones || !earphonesOn)

/** Output devices people wear. A Bluetooth speaker counts too: Android reports it the same way. */
@android.annotation.SuppressLint("InlinedApi") // Only numbers: an older phone just never reports them.
fun isEarphones(type: Int): Boolean = when (type) {
    android.media.AudioDeviceInfo.TYPE_WIRED_HEADSET,
    android.media.AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
    android.media.AudioDeviceInfo.TYPE_USB_HEADSET,
    android.media.AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
    android.media.AudioDeviceInfo.TYPE_BLE_HEADSET,
    android.media.AudioDeviceInfo.TYPE_HEARING_AID,
    -> true
    else -> false
}

/** Android keeps a volume per output, so a saved one is only walked back on the output it came from. */
fun volumeGoesBack(savedOnEarphones: Boolean, earphonesOn: Boolean): Boolean = savedOnEarphones == earphonesOn

/** The most the capture thread waits for an encoded packet: far longer than encoding takes. */
const val ENCODER_WAIT_US = 5_000L

/**
 * How long to wait for the encoder's next packet, in microseconds. A packet that isn't collected when
 * it is ready sits there until the next piece of capture, up to 21 ms later: delay, and a less even
 * stream, for nothing. When no packet can be ready, nothing is waited for.
 */
fun encoderWaitUs(pendingBytes: Long, bytesPerPacket: Long): Long =
    if (pendingBytes >= bytesPerPacket) ENCODER_WAIT_US else 0L

/** Home's card for a share: title and line. */
fun audioShareCard(source: AudioSource, peer: String): Pair<String, String> =
    if (source == AudioSource.MICROPHONE) {
        "Sharing your microphone with $peer" to "Your PC hears this phone's microphone"
    } else {
        "Sharing this phone's audio with $peer" to "What this phone plays goes to your PC"
    }
