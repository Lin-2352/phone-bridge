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
    fun control_waiting_never_without_a_refusal() {
        assertFalse(controlWaiting(refusedAtMs = 0, nowMs = 1_000, controlOn = false))
        assertFalse(controlWaiting(refusedAtMs = 0, nowMs = 60_000, controlOn = false))
    }

    @Test
    fun control_waiting_within_two_minutes_of_a_refusal() {
        val at = 1_700_000_000_000
        assertTrue(controlWaiting(at, nowMs = at, controlOn = false))
        assertTrue(controlWaiting(at, nowMs = at + 119_000, controlOn = false))
        assertTrue(controlWaiting(at, nowMs = at + 120_000, controlOn = false))
    }

    @Test
    fun control_waiting_ends_after_the_window() {
        val at = 1_700_000_000_000
        assertFalse(controlWaiting(at, nowMs = at + 120_001, controlOn = false))
        assertFalse(controlWaiting(at, nowMs = at + 3_600_000, controlOn = false))
        assertTrue(controlWaiting(at, nowMs = at + 200_000, controlOn = false, windowMs = 300_000))
    }

    @Test
    fun control_waiting_is_false_once_control_runs() {
        val at = 1_700_000_000_000
        assertFalse(controlWaiting(at, nowMs = at + 1_000, controlOn = true))
    }

    @Test
    fun a_refusal_in_the_future_does_not_count() {
        // The clock moved back after the refusal: not shown until it would have expired anyway.
        val at = 1_700_000_000_000
        assertFalse(controlWaiting(at, nowMs = at - 1_000, controlOn = false))
    }

    @Test
    fun app_notifications_off_comes_last() {
        assertEquals(listOf(Need.AppNotificationsOff), needs(true, false, true, true, false, false, appNotificationsOn = false))
        assertEquals(listOf(Need.PcForgotPhone, Need.AppNotificationsOff), needs(true, false, true, true, false, true, appNotificationsOn = false))
    }

    /** M3.7 (B5): nothing else on Home works until the identity is reset, so it comes first. */
    @Test fun a_lost_identity_comes_first() {
        assertEquals(
            Need.IdentityLost,
            needs(false, true, false, false, true, true, appNotificationsOn = false, identityLost = true).first(),
        )
        assertTrue(Need.IdentityLost !in needs(true, true, true, true, false, false, appNotificationsOn = true))
    }

    /** L1: a VPN may keep the PC out; the hint is only a hint, and only for a paired phone. */
    @Test fun vpn_hint_shows_only_with_a_vpn_and_a_paired_pc() {
        val text = "A VPN is on. If your PC can't reach this phone, turn the VPN off or allow local network access"
        assertEquals(text, vpnHint(vpnOn = true, paired = true))
        assertEquals(null, vpnHint(vpnOn = false, paired = true))
        assertEquals(null, vpnHint(vpnOn = true, paired = false))
    }

    @Test fun vpn_on_comes_after_every_other_need() {
        val all = needs(false, true, false, false, true, true, appNotificationsOn = false, identityLost = true, vpnOn = true)
        assertEquals(Need.VpnOn, all.last())
        assertEquals(Need.IdentityLost, all.first())
        assertTrue(Need.VpnOn !in needs(true, true, true, true, false, false, appNotificationsOn = true))
    }
}
