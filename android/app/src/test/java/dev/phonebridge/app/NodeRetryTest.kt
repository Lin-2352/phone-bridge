package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class NodeRetryTest {
    @Test
    fun a_failed_start_is_tried_again_with_growing_waits_then_given_up() {
        assertEquals(1_000L, nodeRetryDelayMs(1))
        assertEquals(2_000L, nodeRetryDelayMs(2))
        assertEquals(4_000L, nodeRetryDelayMs(3))
        assertNull(nodeRetryDelayMs(4))
    }

    @Test
    fun the_logged_reason_is_bounded() {
        assertEquals("transport error: Failed to bind sockets", nodeFailureReason("transport error: Failed to bind sockets"))
        assertEquals(120, nodeFailureReason("x".repeat(500)).length)
        assertEquals("unknown", nodeFailureReason(null))
    }
}
