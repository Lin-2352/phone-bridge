package dev.phonebridge.app.audio

/** The phone's volume at the start of a share: it sets the PC's level, so a leftover 0 must not carry over. */
fun shareStartIndex(remembered: Int, max: Int): Int =
    if (remembered <= 0) max / 2 else remembered.coerceAtMost(max)

/** Relative steps that give the phone its own volume back. Negative [saved]: no share was running. */
fun stepsBack(current: Int, saved: Int): Int = if (saved < 0) 0 else saved - current

/** The PC level a muted share starts at. A saved 0 would be silence with nothing on screen to explain it. */
fun muteShareStartStep(saved: Int): Int = if (saved <= 0) PC_LEVEL_STEPS else clampStep(saved)

/** Home's card for a share: title and line. */
fun audioShareCard(source: AudioSource, peer: String): Pair<String, String> =
    if (source == AudioSource.MICROPHONE) {
        "Sharing your microphone with $peer" to "Your PC hears this phone's microphone"
    } else {
        "Sharing this phone's audio with $peer" to "What this phone plays goes to your PC"
    }
