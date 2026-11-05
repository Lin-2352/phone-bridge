package dev.phonebridge.app

/**
 * Log sampling period: only while a PC session is open or Relay is on screen (null: no timer;
 * wait for one of them). Idle with no PC, Relay must not wake the phone (DECISIONS, idle budget).
 */
fun sampleIntervalMs(sessionActive: Boolean, uiVisible: Boolean, interactive: Boolean): Long? = when {
    !sessionActive && !uiVisible -> null
    interactive -> 10_000L
    else -> 60_000L
}

/** "Last seen 5 min ago" only needs redrawing while someone can see it: screen on, PC away. */
fun statusRedrawDue(screenOn: Boolean, ready: Boolean, connected: Boolean): Boolean = screenOn && ready && !connected
