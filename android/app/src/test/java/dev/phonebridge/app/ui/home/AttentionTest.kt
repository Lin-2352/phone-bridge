package dev.phonebridge.app.ui.home

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AttentionTest {
    @Test
    fun nothing_needed_is_empty() {
        assertTrue(needs(notifyAccess = true, screenshotsOn = true, photosGranted = true, controlOn = true, controlWaiting = true, pcForgot = false, appNotificationsOn = true).isEmpty())
        assertTrue(needs(notifyAccess = true, screenshotsOn = false, photosGranted = false, controlOn = false, controlWaiting = false, pcForgot = false, appNotificationsOn = true).isEmpty())
    }

    @Test
    fun priority_order_is_fixed() {
        assertEquals(listOf(Need.NotificationAccess, Need.PcForgotPhone), needs(false, false, true, true, false, true, appNotificationsOn = true))
        assertEquals(
            listOf(Need.NotificationAccess, Need.PhotosPermission, Need.ControlOff, Need.PcForgotPhone, Need.AppNotificationsOff),
            needs(notifyAccess = false, screenshotsOn = true, photosGranted = false, controlOn = false, controlWaiting = true, pcForgot = true, appNotificationsOn = false),
        )
    }

    @Test
    fun photos_only_matter_when_screenshots_are_on() {
        assertEquals(listOf(Need.PhotosPermission), needs(true, screenshotsOn = true, photosGranted = false, controlOn = true, controlWaiting = false, pcForgot = false, appNotificationsOn = true))
        assertTrue(needs(true, screenshotsOn = false, photosGranted = false, controlOn = true, controlWaiting = false, pcForgot = false, appNotificationsOn = true).isEmpty())
        assertTrue(needs(true, screenshotsOn = true, photosGranted = true, controlOn = true, controlWaiting = false, pcForgot = false, appNotificationsOn = true).isEmpty())
    }

    @Test
    fun control_off_only_matters_while_a_request_waits() {
        assertEquals(listOf(Need.ControlOff), needs(true, false, true, controlOn = false, controlWaiting = true, pcForgot = false, appNotificationsOn = true))
        assertTrue(needs(true, false, true, controlOn = false, controlWaiting = false, pcForgot = false, appNotificationsOn = true).isEmpty())
        assertTrue(needs(true, false, true, controlOn = true, controlWaiting = true, pcForgot = false, appNotificationsOn = true).isEmpty())
    }

    @Test
    fun control_off_never_shows_from_todays_source() {
        // Control switches itself off after every session: off alone is the resting state, not a problem.
        for (on in listOf(true, false)) {
            assertFalse(Need.ControlOff in needs(true, false, true, controlOn = on, controlWaiting = PC_CONTROL_WAITING, pcForgot = PC_FORGOT_PHONE, appNotificationsOn = true))
        }
    }

    @Test
    fun app_notifications_off_comes_last() {
        assertEquals(listOf(Need.AppNotificationsOff), needs(true, false, true, true, false, false, appNotificationsOn = false))
        assertEquals(listOf(Need.PcForgotPhone, Need.AppNotificationsOff), needs(true, false, true, true, false, true, appNotificationsOn = false))
    }
}
