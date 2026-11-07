package dev.phonebridge.app.devices

import org.junit.Assert.assertEquals
import org.junit.Test

class ConsentTest {
    @Test
    fun never_denies_without_asking() {
        assertEquals(ConsentStep.AutoDeny, consentStep(Allow.Never, keyguardLocked = false))
        assertEquals("also while locked", ConsentStep.AutoDeny, consentStep(Allow.Never, keyguardLocked = true))
    }

    @Test
    fun always_still_asks_while_locked() {
        assertEquals(ConsentStep.AutoAllow, consentStep(Allow.Always, keyguardLocked = false))
        assertEquals(ConsentStep.Ask, consentStep(Allow.Always, keyguardLocked = true))
    }

    @Test
    fun repeated_never_blocks_do_not_flood_the_timeline() {
        assertEquals(true, shouldNotifyBlock(null, 1_000L))
        assertEquals(false, shouldNotifyBlock(1_000L, 30_000L))
        assertEquals(true, shouldNotifyBlock(1_000L, 61_000L))
        assertEquals("a clock that went back shows it again", true, shouldNotifyBlock(100_000L, 5_000L))
    }

    @Test
    fun ask_asks() {
        assertEquals(ConsentStep.Ask, consentStep(Allow.Ask, keyguardLocked = false))
        assertEquals(ConsentStep.Ask, consentStep(Allow.Ask, keyguardLocked = true))
    }

    @Test
    fun clipboard_goes_only_to_pcs_with_sync_on() {
        val all = mapOf<String, Any?>("a/clipboard_sync" to true, "b/clipboard_sync" to false, "c/other" to true)
        assertEquals(listOf("a"), peersWith(all, listOf("a", "b", "c", "d"), "clipboard_sync"))
    }
}
