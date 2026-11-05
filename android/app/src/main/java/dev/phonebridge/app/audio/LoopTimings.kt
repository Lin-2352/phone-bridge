package dev.phonebridge.app.audio

import java.util.concurrent.atomic.AtomicLong

/** The slowest wait for sound and the slowest handling of a frame since the last look, in ms. */
data class LoopWindow(val maxReadMs: Long, val maxWorkMs: Long)

/**
 * Tells where a stall in the capture loop is: waiting for Android to hand over sound, or handling
 * it (encoding, sending). The worker records, another thread takes.
 */
class LoopTimings {
    private val read = AtomicLong(0)
    private val work = AtomicLong(0)

    fun record(readUs: Long, workUs: Long) {
        read.accumulateAndGet(readUs, ::maxOf)
        work.accumulateAndGet(workUs, ::maxOf)
    }

    fun take() = LoopWindow(read.getAndSet(0) / 1000, work.getAndSet(0) / 1000)
}
