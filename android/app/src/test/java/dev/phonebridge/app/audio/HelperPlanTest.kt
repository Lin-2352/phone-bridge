package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HelperPlanTest {
    private fun facts(
        paired: Boolean = true,
        sdk: Int = 33,
        wifi: Boolean = true,
        alive: Boolean = false,
        canToggle: Boolean = true,
        msSinceAttempt: Long = 10 * 60_000L,
    ) = HelperFacts(paired, sdk, wifi, alive, canToggle, msSinceAttempt)

    @Test
    fun a_paired_phone_with_no_helper_starts_it() {
        assertEquals(HelperStep.START, helperStep(facts()))
    }

    @Test
    fun a_running_helper_needs_nothing() {
        assertEquals(HelperStep.NOTHING, helperStep(facts(alive = true)))
    }

    @Test
    fun an_unpaired_phone_never_touches_wireless_debugging() {
        assertEquals(HelperStep.NOT_SET_UP, helperStep(facts(paired = false)))
    }

    @Test
    fun android_11_and_older_use_normal_capture() {
        assertEquals(HelperStep.UNSUPPORTED, helperStep(facts(sdk = 30)))
    }

    @Test
    fun wireless_debugging_needs_wifi() {
        assertEquals(HelperStep.NO_WIFI, helperStep(facts(wifi = false)))
    }

    /** A lost grant must surface as a state to fix, not as endless attempts. */
    @Test
    fun without_the_settings_grant_it_cannot_start_itself() {
        assertEquals(HelperStep.NEEDS_SETUP_AGAIN, helperStep(facts(canToggle = false)))
    }

    @Test
    fun a_failed_attempt_is_not_retried_quickly() {
        assertEquals(HelperStep.TOO_SOON, helperStep(facts(msSinceAttempt = HELPER_RETRY_MS - 1)))
        assertEquals(HelperStep.START, helperStep(facts(msSinceAttempt = HELPER_RETRY_MS)))
    }

    /** A person asking now is worth a try even just after a failure. */
    @Test
    fun a_request_from_the_pc_ignores_the_wait() {
        assertEquals(HelperStep.START, helperStep(facts(msSinceAttempt = 0), asked = true))
    }

    @Test
    fun the_adb_key_is_renewed_before_android_forgets_it() {
        assertFalse(helperRenewDue(HELPER_RENEW_MS - 1))
        assertTrue(helperRenewDue(HELPER_RENEW_MS))
        assertTrue(HELPER_RENEW_MS < 7L * 24 * 60 * 60 * 1000)
    }

    @Test
    fun wireless_debugging_goes_back_to_how_the_user_had_it() {
        assertTrue(turnWirelessOffAfter(wasOn = false))
        assertFalse(turnWirelessOffAfter(wasOn = true))
    }

    @Test
    fun the_start_command_carries_the_secret_in_the_environment_and_detaches() {
        val c = helperStartCommand("abc123", "/data/app/x==/base.apk")
        assertTrue(c, c.contains("RELAY_HELPER_SECRET='abc123'"))
        assertTrue(c, c.contains("CLASSPATH='/data/app/x==/base.apk'"))
        assertTrue(c, c.contains("dev.phonebridge.helper.Main '/data/app/x==/base.apk'"))
        assertTrue(c, c.contains("nohup"))
        assertTrue(c, c.trimEnd().endsWith("&"))
    }

    /** The command line shows in a process list, the environment does not. */
    @Test
    fun the_secret_is_not_an_argument_of_the_helper() {
        val c = helperStartCommand("abc123", "/a.apk")
        assertFalse(c, c.substringAfter("app_process").contains("abc123"))
    }

    @Test
    fun a_quote_in_the_apk_path_cannot_break_out_of_the_command() {
        val c = helperStartCommand("abc123", "/a'; reboot; '.apk")
        // Each quote closes the string, adds an escaped quote and reopens: all of it stays one word.
        assertTrue(c, c.contains("""'/a'\''; reboot; '\''.apk'"""))
    }

    @Test
    fun a_pairing_code_is_six_digits() {
        assertEquals("123456", pairingCode(" 123456 "))
        assertEquals("123456", pairingCode("123 456"))
        assertNull(pairingCode("12345"))
        assertNull(pairingCode("12345a"))
        assertNull(pairingCode(""))
    }

    @Test
    fun the_settings_row_says_what_is_true() {
        assertEquals("On. No prompt, the phone stays silent, and its volume bar sets the level", helperRowText(HelperStep.NOTHING))
        assertFalse(helperRowText(HelperStep.NOT_SET_UP).startsWith("On"))
        assertTrue(helperRowText(HelperStep.NO_WIFI).contains("Wi-Fi"))
        for (s in HelperStep.entries.filter { it != HelperStep.NOTHING }) {
            assertFalse("$s claims to be on", helperRowText(s).startsWith("On"))
        }
    }
}
