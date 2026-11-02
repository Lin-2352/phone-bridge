package dev.phonebridge.app.ui.home

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AttentionTest {
    @Test
    fun nothing_needed_is_empty() {
        assertTrue(needs(notifyAccess = true, screenshotsOn = true, photosGranted = true, controlOn = true, controlWaiting = true, pcForgot = false).isEmpty())
        assertTrue(needs(notifyAccess = true, screenshotsOn = false, photosGranted = false, controlOn = false, controlWaiting = false, pcForgot = false).isEmpty())
    }

    @Test
    fun priority_order_is_fixed() {
        assertEquals(listOf(Need.NotificationAccess, Need.PcForgotPhone), needs(false, false, true, true, false, true))
        assertEquals(
            listOf(Need.NotificationAccess, Need.PhotosPermission, Need.ControlOff, Need.PcForgotPhone),
            needs(notifyAccess = false, screenshotsOn = true, photosGranted = false, controlOn = false, controlWaiting = true, pcForgot = true),
        )
    }

    @Test
    fun photos_only_matter_when_screenshots_are_on() {
        assertEquals(listOf(Need.PhotosPermission), needs(true, screenshotsOn = true, photosGranted = false, controlOn = true, controlWaiting = false, pcForgot = false))
        assertTrue(needs(true, screenshotsOn = false, photosGranted = false, controlOn = true, controlWaiting = false, pcForgot = false).isEmpty())
        assertTrue(needs(true, screenshotsOn = true, photosGranted = true, controlOn = true, controlWaiting = false, pcForgot = false).isEmpty())
    }

    @Test
    fun control_off_only_matters_while_a_request_waits() {
        assertEquals(listOf(Need.ControlOff), needs(true, false, true, controlOn = false, controlWaiting = true, pcForgot = false))
        assertTrue(needs(true, false, true, controlOn = false, controlWaiting = false, pcForgot = false).isEmpty())
        assertTrue(needs(true, false, true, controlOn = true, controlWaiting = true, pcForgot = false).isEmpty())
    }
}
