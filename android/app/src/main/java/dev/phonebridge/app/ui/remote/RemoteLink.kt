package dev.phonebridge.app.ui.remote

import dev.phonebridge.app.APP_NAME

/** The touchpad's link to the PC: what the status line says and whether Retry shows. */
enum class LinkPhase { Connecting, Connected, Lost, Failed, Off }

/**
 * The FFI's input calls return false when the session is gone (stopped, or the PC closed it). Only an
 * explicit false while connected means the link dropped; a late gesture after a failure changes nothing.
 */
fun linkAfterSend(phase: LinkPhase, sent: Boolean): LinkPhase =
    if (!sent && phase == LinkPhase.Connected) LinkPhase.Lost else phase

fun showRetry(phase: LinkPhase): Boolean = phase == LinkPhase.Lost || phase == LinkPhase.Failed || phase == LinkPhase.Off

/** The status line for [phase]; Failed and Off carry their own sentence (see [startFailureText]). */
fun linkText(phase: LinkPhase, peer: String): String = when (phase) {
    LinkPhase.Connecting -> "Connecting\u2026"
    LinkPhase.Connected -> "Controlling $peer"
    LinkPhase.Lost -> "Connection to $peer lost. Retry to reconnect."
    LinkPhase.Failed, LinkPhase.Off -> ""
}

/** Why the input session could not start, with what to do. */
fun startFailureText(peer: String, reason: String?): String =
    if (reason?.contains("declined") == true) {
        "$peer did not allow control. Turn on \u201cLet my phone control this PC\u201d in $APP_NAME settings on the PC."
    } else {
        "Could not reach $peer. Is $APP_NAME running there?"
    }

/** A key tap: press, then release. False as soon as one half is refused. */
fun pressAndRelease(send: (Boolean) -> Boolean): Boolean = send(true) && send(false)
