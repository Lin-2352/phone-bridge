package dev.phonebridge.app.audio

import kotlin.math.pow

/** Steps of the PC's level, which the phone's keys move while Relay holds its speaker silent. */
const val PC_LEVEL_STEPS = 16
private const val DB_PER_STEP = 3f

fun clampStep(step: Int): Int = step.coerceIn(0, PC_LEVEL_STEPS)

/** 3 dB a step, roughly what a phone's own volume keys do; step 0 is silence. */
fun gainForStep(step: Int): Float = when (val s = clampStep(step)) {
    0 -> 0f
    PC_LEVEL_STEPS -> 1f
    else -> 10f.pow(-(PC_LEVEL_STEPS - s) * DB_PER_STEP / 20f)
}

/**
 * The PC level step in a report that the phone's volume moved from [prev] to [index] while it is held
 * at [held]. Only a move away is a press: a move back is Relay undoing one.
 */
fun keyStep(held: Int, prev: Int, index: Int): Int =
    if (kotlin.math.abs(index - held) > kotlin.math.abs(prev - held)) (index - prev).coerceIn(-1, 1) else 0

/** The one step that brings the volume back toward [held]. */
fun undoStep(held: Int, index: Int): Int = (held - index).coerceIn(-1, 1)

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
