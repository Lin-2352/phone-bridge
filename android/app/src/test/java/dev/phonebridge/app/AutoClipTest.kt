package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class AutoClipTest {
    @Test
    fun a_copy_wakes_the_reader_only_when_sharing_is_on_the_phone_is_awake_and_the_overlay_is_allowed() {
        assertEquals(true, shouldAutoRead(syncOn = true, interactive = true, overlay = true, lastWakeMs = 0, nowMs = 1000))
        assertEquals(false, shouldAutoRead(syncOn = false, interactive = true, overlay = true, lastWakeMs = 0, nowMs = 1000))
        assertEquals(false, shouldAutoRead(syncOn = true, interactive = false, overlay = true, lastWakeMs = 0, nowMs = 1000))
        assertEquals(false, shouldAutoRead(syncOn = true, interactive = true, overlay = false, lastWakeMs = 0, nowMs = 1000))
    }

    @Test
    fun two_changes_inside_a_moment_wake_it_once() {
        assertEquals(false, shouldAutoRead(true, true, true, lastWakeMs = 1000, nowMs = 1000 + AUTO_READ_GAP_MS - 1))
        assertEquals(true, shouldAutoRead(true, true, true, lastWakeMs = 1000, nowMs = 1000 + AUTO_READ_GAP_MS))
    }
}
