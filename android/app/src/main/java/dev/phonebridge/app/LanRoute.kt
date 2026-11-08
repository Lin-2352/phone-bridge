package dev.phonebridge.app

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.util.Log

/** What the routing rule needs to know about one network. */
data class NetSnapshot(
    val id: Long,
    val wifi: Boolean = false,
    val ethernet: Boolean = false,
    val cellular: Boolean = false,
    val vpn: Boolean = false,
)

/**
 * The network Relay should use for the local network while a VPN is up: the Wi-Fi (else the Ethernet) network
 * that is not itself a tunnel. Null when no VPN is up (nothing changes for people without one) or when the phone
 * has no Wi-Fi or Ethernet (a PC cannot be reached over mobile data anyway). Relay's relay servers are off, so
 * everything it sends is local-network traffic.
 */
fun lanNetworkToBind(nets: List<NetSnapshot>): Long? {
    if (nets.none { it.vpn }) return null
    val lan = nets.filter { !it.vpn }
    return (lan.firstOrNull { it.wifi } ?: lan.firstOrNull { it.ethernet })?.id
}

/** The text of the "wants to send" question: tapping the body does not answer it, so it says where the answer is. */
fun offerNotificationText(sizeBytes: ULong): String =
    "%.1f MB. Swipe down for Accept or Decline".format(sizeBytes.toDouble() / 1_048_576)

/** The "did not receive" notification: names the VPN when one is on, since it is the most likely reason a PC cannot be reached. */
fun notReceivedText(peerName: String, fileName: String, vpnOn: Boolean): String =
    if (vpnOn) "$peerName did not receive $fileName. A VPN is on and may be blocking the local network: allow local network access (or exclude $APP_NAME) in the VPN app, or turn it off."
    else "$peerName did not receive $fileName. Is $APP_NAME running there?"

/**
 * While a VPN is up, Relay's own traffic goes to the Wi-Fi network directly (`bindProcessToNetwork`): this is an
 * app-level route, not a change to the VPN or to any setting. Android only honours it when the VPN app allows
 * bypass; the answer is logged so a refusal is visible. Sockets made before the binding keep their old route, so
 * the caller binds before the node starts and restarts the node when this reports a change.
 */
object LanRoute {
    private const val TAG = "PhoneBridge"
    private var boundId: Long? = null

    /** True when the binding changed (the node's sockets then need to be made again). */
    @Synchronized
    fun refresh(ctx: Context): Boolean = runCatching {
        val cm = ctx.getSystemService(ConnectivityManager::class.java)
        val nets = cm.allNetworks.mapNotNull { n ->
            val c = cm.getNetworkCapabilities(n) ?: return@mapNotNull null
            NetSnapshot(
                id = n.networkHandle,
                wifi = c.hasTransport(NetworkCapabilities.TRANSPORT_WIFI),
                ethernet = c.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET),
                cellular = c.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR),
                vpn = c.hasTransport(NetworkCapabilities.TRANSPORT_VPN),
            ) to n
        }
        val want = lanNetworkToBind(nets.map { it.first })
        if (want == boundId) return@runCatching false
        val target = want?.let { id -> nets.firstOrNull { it.first.id == id }?.second }
        val ok = cm.bindProcessToNetwork(target)
        Log.i(TAG, "LANROUTE bound=${target != null && ok} request=${if (target != null) "wifi/ethernet" else "default"} ok=$ok vpn=${nets.any { it.first.vpn }}")
        AppLog.event("events", "lan_route", "bound" to (target != null && ok), "vpn" to nets.any { it.first.vpn })
        boundId = want
        true
    }.getOrElse { Log.i(TAG, "LANROUTE failed: ${it.message}"); false }
}
