package dev.phonebridge.app.audio

import kotlin.math.pow

/** Steps of the PC's level, which the phone's keys move while Relay holds its speaker silent. */
const val PC_LEVEL_STEPS = 16
private const val DB_PER_STEP = 3f

fun clampStep(step: Int): Int = step.coerceIn(0, PC_LEVEL_STEPS)

/** The PC level for the phone's own volume. Rounds up, so only a real 0 is silent. */
fun stepForIndex(index: Int, max: Int): Int =
    if (max <= 0) PC_LEVEL_STEPS else clampStep((index.coerceAtLeast(0) * PC_LEVEL_STEPS + max - 1) / max)

/** The phone's volume that shows the PC level [step]. */
fun indexForStep(step: Int, max: Int): Int =
    (clampStep(step) * max.coerceAtLeast(0) + PC_LEVEL_STEPS / 2) / PC_LEVEL_STEPS

/** The level after a volume key. [direction]: 1 up, -1 down, 0 only shows the bar. */
fun stepAfterKey(current: Int, direction: Int): Int = clampStep(current + direction)

/** 3 dB a step, roughly what a phone's own volume keys do; step 0 is silence. */
fun gainForStep(step: Int): Float = when (val s = clampStep(step)) {
    0 -> 0f
    PC_LEVEL_STEPS -> 1f
    else -> 10f.pow(-(PC_LEVEL_STEPS - s) * DB_PER_STEP / 20f)
}

/**
 * The level of what the PC hears, applied to captured sound before it is encoded. [step] is set by
 * the keys on the main thread, [apply] runs on the capture thread. A change fades across one frame:
 * a jump between two samples would click.
 */
class PcLevel(start: Int) {
    @Volatile var step: Int = clampStep(start)
        set(v) { field = clampStep(v) }

    private var current = gainForStep(start)

    fun adjust(direction: Int) {
        step += direction
    }

    /** Scales 16-bit little-endian PCM in place. */
    fun apply(buf: ByteArray, len: Int, channels: Int) {
        val target = gainForStep(step)
        val from = current
        if (from == 1f && target == 1f) return
        val frames = len / 2 / channels
        if (frames == 0) return
        for (f in 0 until frames) {
            val gain = from + (target - from) * (f + 1) / frames
            for (c in 0 until channels) {
                val at = (f * channels + c) * 2
                val v = (buf[at].toInt() and 0xff) or (buf[at + 1].toInt() shl 8)
                val out = (v * gain).toInt()
                buf[at] = out.toByte()
                buf[at + 1] = (out shr 8).toByte()
            }
        }
        current = target
    }
}
