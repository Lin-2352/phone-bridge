package dev.phonebridge.app

import dev.phonebridge.app.ui.home.pcStatus

/**
 * The one answer to "how is Relay doing with my PC", shared by the status notification, the Quick
 * Settings tile and panel, the widget and (through [pcStatus]) Home.
 */
data class RelayStatus(val title: String, val detail: String?) {
    /** Both parts on one line (widget). */
    val line: String get() = if (detail == null) title else "$title \u00b7 ${detail.replaceFirstChar { it.lowercase() }}"
}

/**
 * [running] the service runs; [ready] its node is up (until then the PC is unknown); [connected] is
 * `widgetConnected`. Relay connects only when there is something to do, so having no open session says
 * nothing about the PC: "not reachable" is only for [attemptFailed], a real attempt that failed.
 */
fun relayStatus(
    running: Boolean, ready: Boolean, pc: String?, connected: Boolean,
    lastSeenMs: Long, nowMs: Long, controller: String? = null, discoveryOff: Boolean = false,
    attemptFailed: Boolean = false,
): RelayStatus = when {
    !running -> RelayStatus("$APP_NAME is off", null)
    !ready -> RelayStatus("Starting\u2026", null)
    controller != null -> RelayStatus("$controller is controlling this phone", null)
    pc == null -> RelayStatus("Not paired yet", null)
    connected -> RelayStatus("Connected to $pc", null)
    discoveryOff -> RelayStatus("$pc not reachable", DISCOVERY_OFF_TEXT)
    attemptFailed -> RelayStatus("$pc not reachable", if (lastSeenMs > 0L) pcStatus(false, lastSeenMs, nowMs, paired = true) else null)
    lastSeenMs <= 0L -> RelayStatus("Paired with $pc", null)
    else -> RelayStatus("Paired with $pc", pcStatus(false, lastSeenMs, nowMs, paired = true))
}

/** Said while the PC is away and another app holds the discovery port. */
const val DISCOVERY_OFF_TEXT = "Can't look for it on a new network: another app uses port 47199"
