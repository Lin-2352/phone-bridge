package dev.phonebridge.app.audio

import java.util.concurrent.atomic.AtomicInteger

/**
 * One capture's end, decided once: either the capture ended by itself ([onSourceEnded] wins) or a
 * stop tore it down ([beginTeardown] wins). The loser does nothing, so a capture that ends while a
 * stop is already running never waits for the share's lock or stops the next share.
 */
class CaptureLifecycle {
    private val state = AtomicInteger(RUNNING)

    /** True for the first end of either kind. */
    fun beginTeardown(): Boolean = state.compareAndSet(RUNNING, TORN_DOWN)

    /** True when the capture's own end is the first: the owner must then stop the share. */
    fun onSourceEnded(): Boolean = state.compareAndSet(RUNNING, SOURCE_ENDED)

    private companion object {
        const val RUNNING = 0
        const val SOURCE_ENDED = 1
        const val TORN_DOWN = 2
    }
}
