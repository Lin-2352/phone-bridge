package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LanRouteTest {
    private val wifi = NetSnapshot(id = 1, wifi = true)
    private val cell = NetSnapshot(id = 2, cellular = true)
    private val vpn = NetSnapshot(id = 3, vpn = true)
    private val ethernet = NetSnapshot(id = 4, ethernet = true)

    @Test fun noVpnMeansNoBinding() = assertNull(lanNetworkToBind(listOf(wifi, cell)))

    @Test fun vpnWithWifiBindsTheWifi() = assertEquals(1L, lanNetworkToBind(listOf(cell, vpn, wifi)))

    @Test fun vpnWithoutAnyLanNetworkBindsNothing() = assertNull(lanNetworkToBind(listOf(cell, vpn)))

    @Test fun ethernetCountsAsLanAndWifiIsPreferred() {
        assertEquals(4L, lanNetworkToBind(listOf(vpn, ethernet)))
        assertEquals(1L, lanNetworkToBind(listOf(vpn, ethernet, wifi)))
    }

    @Test fun aVpnOverWifiIsNotTheLanNetwork() {
        // The tunnel itself reports Wi-Fi as its transport on some phones: never bind to the tunnel.
        val tunnelOverWifi = NetSnapshot(id = 9, wifi = true, vpn = true)
        assertNull(lanNetworkToBind(listOf(tunnelOverWifi)))
        assertEquals(1L, lanNetworkToBind(listOf(tunnelOverWifi, wifi)))
    }

    @Test fun aFailedSendNamesTheVpnWhenOneIsOn() {
        val t = notReceivedText("My PC", "a.pdf", vpnOn = true)
        assertTrue(t, "VPN" in t && "My PC" in t && "a.pdf" in t)
    }

    @Test fun theOfferNotificationSaysWhereTheAnswerIs() {
        val t = offerNotificationText(300_000UL)
        assertTrue(t, "0.3 MB" in t && "Accept" in t && "Decline" in t)
    }

    @Test fun aFailedSendStaysPlainWithoutAVpn() =
        assertEquals("My PC did not receive a.pdf. Is Relay running there?", notReceivedText("My PC", "a.pdf", vpnOn = false))
}
