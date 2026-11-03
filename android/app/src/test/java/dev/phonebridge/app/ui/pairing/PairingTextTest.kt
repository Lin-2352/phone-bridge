package dev.phonebridge.app.ui.pairing

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingTextTest {
    private fun join(r: String?) = joinErrorText(r)

    @Test fun expired_or_used_code_says_get_a_new_one() {
        val t = join("pairing refused: the code expired or was already used, or the other device declined")
        assertTrue(t, t.startsWith("That code has expired"))
        assertTrue(t, "Pair new device" in t)
        assertTrue(join("pairing failed: pairing code expired").startsWith("That code has expired"))
        assertTrue(join("pairing failed: no pairing code is active").startsWith("That code has expired"))
    }

    @Test fun wrong_or_burned_code() {
        assertTrue(join("pairing failed: wrong pairing code").startsWith("Your PC did not accept"))
        assertTrue(join("pairing failed: too many wrong attempts; pairing code burned").startsWith("Your PC did not accept"))
    }

    @Test fun not_a_relay_code() {
        assertTrue(join("not a Relay pairing code").startsWith("That is not a Relay"))
        assertTrue(join("pairing failed: malformed pairing message").startsWith("That is not a Relay"))
    }

    @Test fun unreachable_pc() {
        for (r in listOf("timed out", "transport error: connect failed", "connection closed")) {
            val t = join(r)
            assertTrue("$r -> $t", t.startsWith("Could not reach your PC"))
            assertTrue(t, "same Wi-Fi" in t || "same network" in t)
        }
    }

    @Test fun unknown_reason_still_names_a_next_step_and_never_leaks_the_raw_text() {
        val t = join("quic: weird internal thing 0x7f")
        assertFalse(t, "0x7f" in t)
        assertTrue(t, "scan" in t.lowercase())
        assertEquals(t, join(null))
    }

    @Test fun confirm_failures() {
        assertTrue(confirmErrorText(true, "no pairing in progress").startsWith("Pairing was cancelled"))
        assertTrue(confirmErrorText(true, "the other device declined").startsWith("Your PC did not confirm"))
        assertTrue(confirmErrorText(true, "timed out").startsWith("Lost contact"))
        assertTrue(confirmErrorText(true, "connection closed").startsWith("Lost contact"))
        assertTrue(confirmErrorText(true, "pairing refused: the code expired or was already used, or the other device declined").startsWith("Your PC did not confirm"))
        // The user said No: nothing failed, so no error wording.
        assertEquals("Pairing cancelled. Nothing was paired.", confirmErrorText(false, "anything"))
    }

    @Test fun progress_text_gets_more_honest_when_slow() {
        assertEquals("Contacting your PC\u2026", pairingProgressText(0))
        assertEquals("Contacting your PC\u2026", pairingProgressText(9_999))
        assertTrue(pairingProgressText(10_000).startsWith("Still trying"))
        assertTrue(pairingProgressText(30_000).contains("Cancel"))
    }

    @Test fun typed_code_validation() {
        val id = "ab".repeat(32)
        val tok = "cd".repeat(16)
        assertTrue(looksLikePairingCode("pb1:$id:$tok:10.0.0.2:47200,192.168.1.2:47200"))
        assertTrue(looksLikePairingCode("  pb1:$id:$tok:10.0.0.2:47200\n"))
        assertTrue(looksLikePairingCode("pb1:$id:$tok:"))
        assertFalse(looksLikePairingCode(""))
        assertFalse(looksLikePairingCode("https://example.com"))
        assertFalse(looksLikePairingCode("pb1:abab"))
        assertFalse(looksLikePairingCode("pb1:${"zz".repeat(32)}:$tok:"))
        assertFalse(looksLikePairingCode("pb1:${"ab".repeat(31)}:$tok:"))
    }

    @Test fun non_relay_qr_hint_needs_a_second_of_the_same_view() {
        val t = NotRelayTracker()
        assertFalse(t.onDecode("https://example.com", 0))
        assertFalse(t.onDecode("https://example.com", 500))
        assertTrue(t.onDecode("https://example.com", 1_000))
        // The code leaves the view: after a gap the hint goes away and the timer restarts.
        assertTrue(t.onDecode(null, 1_500))
        assertFalse(t.onDecode(null, 3_000))
        assertFalse(t.onDecode("other", 3_100))
        assertTrue(t.onDecode("other", 4_100))
    }

    @Test fun camera_denial_picks_the_right_recovery() {
        assertEquals(DenialAction.TryAgain, denialAction(shouldShowRationale = true))
        assertEquals(DenialAction.OpenSettings, denialAction(shouldShowRationale = false))
    }
}
