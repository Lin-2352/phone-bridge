package dev.phonebridge.app

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities

/** Whether any network the phone has is a VPN (a split tunnel may still reach the LAN, so this is a hint only). */
fun vpnActive(ctx: Context): Boolean = runCatching {
    val cm = ctx.getSystemService(ConnectivityManager::class.java)
    cm.allNetworks.any { cm.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true }
}.getOrDefault(false)
