package dev.phonebridge.app

/** Android 17's runtime permission for talking to devices on the local network. */
const val ACCESS_LOCAL_NETWORK = "android.permission.ACCESS_LOCAL_NETWORK"

/**
 * Apps targeting Android 17 (API 37) need [ACCESS_LOCAL_NETWORK] for any LAN traffic, which is all
 * of Relay (QUIC to the PC, discovery broadcasts). Asked only once Relay targets 37 (A9).
 */
fun needsLocalNetworkRequest(sdkInt: Int, targetSdk: Int, granted: Boolean): Boolean =
    sdkInt >= 37 && targetSdk >= 37 && !granted
