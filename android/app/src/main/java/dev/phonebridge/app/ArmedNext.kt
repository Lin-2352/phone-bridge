package dev.phonebridge.app

/** What Instant screen does after restarting its kept capture for a PC's request. */
enum class ArmedNext { SHARE, BACK_TO_IDLE, DISARM }

/**
 * A failed restart gives the capture up. A restart that got no session (the PC went away or the
 * request was refused) must put the encoder back to idle: it was started for this request only.
 */
fun armedAfterAnswer(restartOk: Boolean, answered: Boolean): ArmedNext = when {
    !restartOk -> ArmedNext.DISARM
    !answered -> ArmedNext.BACK_TO_IDLE
    else -> ArmedNext.SHARE
}
