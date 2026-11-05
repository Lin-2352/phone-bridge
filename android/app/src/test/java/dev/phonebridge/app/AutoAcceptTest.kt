package dev.phonebridge.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AutoAcceptTest {
    @Test fun alwaysAllowedIsNotAutoAcceptedWhileLocked() {
        assertFalse(shouldAutoAccept(alwaysAllowed = true, keyguardLocked = true))
        assertTrue(shouldAutoAccept(alwaysAllowed = true, keyguardLocked = false))
        assertFalse(shouldAutoAccept(alwaysAllowed = false, keyguardLocked = false))
    }

    @Test fun inputIsPausedWhileLockedOrAPromptIsUp() {
        assertFalse(inputAllowed(keyguardLocked = true, consentActive = false))
        assertFalse(inputAllowed(keyguardLocked = false, consentActive = true))
        assertTrue(inputAllowed(keyguardLocked = false, consentActive = false))
    }

    @Test fun allowFromNotificationOpensTheQuestionBelowAndroid12() {
        assertTrue(allowActionOpensActivity(29))
        assertTrue(allowActionOpensActivity(30))
        assertFalse(allowActionOpensActivity(31))
    }
}
