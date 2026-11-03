package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ScreenTextTest {
    @Test fun notificationSaysViewOnlyUnlessControlIsOn() {
        assertEquals("Your PC can see and use this screen", screenShareText(controlOn = true, inputIgnored = false))
        assertEquals("Your PC can see this screen (view only)", screenShareText(controlOn = false, inputIgnored = false))
    }

    @Test fun ignoredInputIsSaidOnThePhone() {
        val t = screenShareText(controlOn = false, inputIgnored = true)
        assertTrue(t, t.contains("touch") && t.contains("ignored"))
    }

    @Test fun consentDoesNotPromiseTouchWithoutControl() {
        assertTrue(screenConsentBody(true).contains("tap and type"))
        assertFalse(screenConsentBody(false).contains("tap and type"))
        assertTrue(screenConsentBody(false).contains("view only"))
    }

    @Test fun pcNoticeNamesTheFix() {
        assertTrue(controlOffNotice().contains("Relay control"))
    }
}
