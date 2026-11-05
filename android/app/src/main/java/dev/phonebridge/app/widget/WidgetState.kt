package dev.phonebridge.app.widget

import dev.phonebridge.app.relayStatus

/**
 * The widget's status line: the shared [relayStatus] on one line. [running] means the service runs and
 * its node is up; [starting] the service runs but the node is not up yet.
 */
fun widgetLabel(running: Boolean, pcName: String?, connected: Boolean, starting: Boolean = false, lastSeenMs: Long = 0L, nowMs: Long = 0L, attemptFailed: Boolean = false): String =
    relayStatus(running || starting, running, pcName, connected, lastSeenMs, nowMs, attemptFailed = attemptFailed).line

/**
 * Connected as Home means it (ui/home/HomeTab.kt `live`, worded "Connected" by `pcStatus`): only while
 * the PC really has a session open with this phone, or controls it, or sees its screen or camera.
 */
fun widgetConnected(running: Boolean, pc: String?, sessions: Int, controlledBy: String?, screenTo: String?, cameraTo: String?, audioWith: String? = null): Boolean =
    running && pc != null && (sessions > 0 || controlledBy != null || screenTo != null || cameraTo != null || audioWith != null)
