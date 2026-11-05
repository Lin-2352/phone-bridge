package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class OutdatedTextTest {
    // The reasons as Rust's transport error prints them (pb-transport Error::VersionMismatch).
    private fun reason(text: String) = "reason=$text"

    @Test
    fun an_older_pc_is_named() {
        assertEquals(
            "DESK runs an older version of $APP_NAME. Update $APP_NAME on DESK",
            outdatedText(reason("the other device runs an older version of Relay: update Relay there"), "DESK"),
        )
    }

    @Test
    fun an_older_phone_says_update_here() {
        assertEquals(
            "Update $APP_NAME on this phone: DESK needs a newer version",
            outdatedText(reason("this device runs an older version of Relay than the other device: update Relay here"), "DESK"),
        )
    }

    @Test
    fun an_unknown_side_says_both() {
        assertEquals(
            "DESK runs a different version of $APP_NAME. Update $APP_NAME on both devices",
            outdatedText(reason("the other device runs a different version of Relay: update both to the latest version"), "DESK"),
        )
    }

    @Test
    fun other_failures_keep_their_own_text() {
        assertNull(outdatedText(reason("timed out"), "DESK"))
        assertNull(outdatedText(null, "DESK"))
    }
}
