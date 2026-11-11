package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PullClipTest {
    @Test
    fun a_pull_is_answered_only_when_the_phone_can_open_a_window_and_read() {
        assertNull(pullRefusal(interactive = true, locked = false, overlay = true))
        assertEquals("The phone's screen is off. Wake it and try again", pullRefusal(false, false, true))
        assertEquals("The phone is locked. Unlock it and try again", pullRefusal(true, true, true))
        assertEquals(
            "Allow Display over other apps for Relay on the phone so a PC can ask for its clipboard",
            pullRefusal(true, false, overlay = false),
        )
    }

    @Test
    fun text_goes_out_and_a_picture_goes_out_when_asked_for_and_allowed() {
        assertEquals(PullAction.SendText, pullAction(hasText = true, hasImage = false, wantImage = false, imagesOn = false, sensitive = false, sendSensitive = false))
        assertEquals(PullAction.SendImage, pullAction(hasText = false, hasImage = true, wantImage = true, imagesOn = true, sensitive = false, sendSensitive = false))
    }

    @Test
    fun an_image_the_pc_did_not_ask_for_or_the_phone_does_not_share_is_not_sent() {
        assertEquals(
            PullAction.Refuse("A picture is copied, but picture sharing is off"),
            pullAction(hasText = false, hasImage = true, wantImage = false, imagesOn = true, sensitive = false, sendSensitive = false),
        )
        assertEquals(
            PullAction.Refuse("A picture is copied, but picture sharing is off"),
            pullAction(hasText = false, hasImage = true, wantImage = true, imagesOn = false, sensitive = false, sendSensitive = false),
        )
    }

    @Test
    fun private_text_stays_on_the_phone_unless_passwords_and_codes_is_on() {
        assertEquals(
            PullAction.Refuse("That text is marked private; turn on Passwords and codes to send it"),
            pullAction(hasText = true, hasImage = false, wantImage = true, imagesOn = true, sensitive = true, sendSensitive = false),
        )
        assertEquals(PullAction.SendText, pullAction(true, false, true, true, sensitive = true, sendSensitive = true))
    }

    @Test
    fun nothing_copied_says_so() {
        assertEquals(PullAction.Refuse("Nothing is copied on the phone"), pullAction(false, false, true, true, false, false))
    }
}
