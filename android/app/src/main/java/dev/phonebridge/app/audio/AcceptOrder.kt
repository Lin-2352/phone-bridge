package dev.phonebridge.app.audio

/** What a share does about telling the PC "yes" while the phone's speaker is being silenced. */
enum class AcceptStep { WAIT, ANSWER }

/**
 * The PC hears "yes" once the phone is [silenced], or once [waitedMs] reaches [limitMs] (a busy volume
 * queue must not hold a share back). Neither case waits on the main thread.
 */
fun acceptOrder(silenced: Boolean, waitedMs: Long, limitMs: Long): AcceptStep =
    if (silenced || waitedMs >= limitMs) AcceptStep.ANSWER else AcceptStep.WAIT
