package dev.phonebridge.app.widget

import dev.phonebridge.app.APP_NAME

/**
 * The widget's status line. [running] means the service runs and its node is up; [pcName] is the
 * paired PC (null when none); [connected] is [widgetConnected].
 */
fun widgetLabel(running: Boolean, pcName: String?, connected: Boolean): String = when {
    !running -> "$APP_NAME is off"
    pcName == null -> "Not paired"
    connected -> "Connected to $pcName"
    else -> "$pcName not reachable"
}

/**
 * Connected as Home means it (ui/home/HomeTab.kt `live`, worded "Connected" by `pcStatus`): only while
 * the PC really has a session open with this phone, or controls it, or sees its screen or camera.
 */
fun widgetConnected(running: Boolean, pc: String?, sessions: Int, controlledBy: String?, screenTo: String?, cameraTo: String?): Boolean =
    running && pc != null && (sessions > 0 || controlledBy != null || screenTo != null || cameraTo != null)
