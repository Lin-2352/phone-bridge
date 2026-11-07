package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DisconnectTest {
    private val before: Map<String, Any?> = mapOf(
        "allow_control_pcs" to setOf("pc1"),
        "allow_screen_pcs" to setOf("pc1", "pc2"),
        "allow_camera_pcs" to setOf("pc2"),
        "audio_skip_confirm_phone_to_pc" to true,
        "enabled" to true,
        "theme" to 2,
        "clipboard_sync" to true,
    )

    @Test
    fun disconnect_clears_every_always_allow() {
        val after = afterDisconnect(before)
        for (key in alwaysAllowKeysToClear()) {
            assertFalse("$key is gone", after.containsKey(key))
        }
        assertEquals(7, alwaysAllowKeysToClear().size)
        assertEquals("the global automatic accept shows off too", false, after["auto_accept"])
        // Other settings are left alone.
        assertEquals(2, after["theme"])
        assertEquals(true, after["clipboard_sync"])
    }

    @Test
    fun disconnect_clears_every_pcs_auto_accept_and_audio_always() {
        val m = before + mapOf(
            "allow_audio_pcs" to setOf("pc1"),
            "pc1/auto_accept" to true,
            "pc2/auto_accept" to true,
            "pc1/never_screen" to true,
            "pc1/clipboard_sync" to true,
        )
        val after = afterDisconnect(m)
        assertFalse(after.containsKey("allow_audio_pcs"))
        assertFalse(after.containsKey("pc1/auto_accept"))
        assertFalse(after.containsKey("pc2/auto_accept"))
        // Never and the clipboard switch are not "always allow": they stay.
        assertEquals(true, after["pc1/never_screen"])
        assertEquals(true, after["pc1/clipboard_sync"])
    }

    @Test
    fun a_revocation_also_clears_the_legacy_key() {
        val m = before + mapOf("allow_control" to true, "allow_screen" to true)
        val after = afterDisconnect(m)
        assertFalse(after.containsKey("allow_control"))
        assertFalse(after.containsKey("allow_screen"))
    }

    @Test
    fun disconnect_leaves_relay_paused_until_resumed() {
        val paused = afterDisconnect(before)
        assertEquals(false, paused["enabled"])
        // Resuming turns Relay on again; it does not bring the "always allow" answers back.
        val resumed = afterResume(paused)
        assertEquals(true, resumed["enabled"])
        for (key in alwaysAllowKeysToClear()) assertFalse(resumed.containsKey(key))
        assertTrue(resumed["clipboard_sync"] == true)
    }
}
