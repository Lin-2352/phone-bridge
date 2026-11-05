package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Test

class LoopTimingsTest {
    @Test
    fun the_slowest_read_and_frame_are_kept_and_restart_when_taken() {
        val t = LoopTimings()
        t.record(readUs = 9_000, workUs = 300)
        t.record(readUs = 650_000, workUs = 200)
        t.record(readUs = 10_000, workUs = 40_000)
        assertEquals(LoopWindow(maxReadMs = 650, maxWorkMs = 40), t.take())
        assertEquals(LoopWindow(0, 0), t.take())
    }
}
