package dev.phonebridge.app.audio

/** The phone's volume at the start of a share: it sets the PC's level, so a leftover 0 must not carry over. */
fun shareStartIndex(remembered: Int, max: Int): Int =
    if (remembered <= 0) max / 2 else remembered.coerceAtMost(max)

/** Relative steps that give the phone its own volume back. Negative [saved]: no share was running. */
fun stepsBack(current: Int, saved: Int): Int = if (saved < 0) 0 else saved - current
