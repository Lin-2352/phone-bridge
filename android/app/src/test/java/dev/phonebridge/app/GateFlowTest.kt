package dev.phonebridge.app

import dev.phonebridge.app.security.Grace
import dev.phonebridge.app.security.GateStep
import dev.phonebridge.app.security.gateStep
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class GateFlowTest {
    private var now = 1_000L
    private fun grace() = Grace(windowMs = 60_000, clock = { now })

    @Test
    fun asks_the_first_time() {
        assertTrue(grace().needsUnlock())
    }

    @Test
    fun no_ask_within_60_s() {
        val g = grace()
        g.passed()
        now += 59_999
        assertFalse(g.needsUnlock())
    }

    @Test
    fun asks_again_at_60_s() {
        val g = grace()
        g.passed()
        now += 60_000
        assertTrue(g.needsUnlock())
    }

    @Test
    fun a_failed_unlock_starts_no_grace() {
        val g = grace()
        // A failed or cancelled attempt never calls passed(): the next action asks again at once.
        now += 5
        assertTrue(g.needsUnlock())
    }

    @Test
    fun locking_the_screen_resets_the_grace() {
        val g = grace()
        g.passed()
        g.reset()
        assertTrue(g.needsUnlock())
    }

    @Test
    fun a_clock_that_goes_backwards_asks() {
        val g = grace()
        g.passed()
        now -= 10_000
        assertTrue("a clock that went backwards cannot prove the grace", g.needsUnlock())
    }

    @Test
    fun gate_off_means_proceed() {
        assertEquals(GateStep.Proceed, gateStep(enabled = false, available = true, graceOk = false))
    }

    @Test
    fun unavailable_means_proceed_with_reason() {
        // The gate is on but cannot run (the screen lock was removed): the old consent still stands.
        assertEquals(GateStep.ProceedNoGate, gateStep(enabled = true, available = false, graceOk = false))
    }

    @Test
    fun gate_on_asks_unless_the_grace_holds() {
        assertEquals(GateStep.Prompt, gateStep(enabled = true, available = true, graceOk = false))
        assertEquals(GateStep.Proceed, gateStep(enabled = true, available = true, graceOk = true))
    }
}
