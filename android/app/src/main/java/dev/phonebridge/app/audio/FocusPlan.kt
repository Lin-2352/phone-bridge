package dev.phonebridge.app.audio

/** What happened to PC audio's place on the phone's output. */
enum class FocusEvent { GAIN, LOSS, LOSS_TRANSIENT, LOSS_CAN_DUCK, NOISY }

/** `AudioManager.AUDIOFOCUS_*` change codes; anything else is ignored. */
fun focusEventOf(code: Int): FocusEvent? = when (code) {
    1 -> FocusEvent.GAIN
    -1 -> FocusEvent.LOSS
    -2 -> FocusEvent.LOSS_TRANSIENT
    -3 -> FocusEvent.LOSS_CAN_DUCK
    else -> null
}

sealed class PlayerAction {
    data object Play : PlayerAction()

    /** Live audio: silent until focus comes back (nothing to catch up on). */
    data object Pause : PlayerAction()

    data class Duck(val gain: Float) : PlayerAction()

    /** Stop playback and tell the PC. */
    data object End : PlayerAction()
}

/** A call pauses PC audio, another player ends it, a short sound ducks it; unplugged headphones end it. */
fun playerReaction(e: FocusEvent): PlayerAction = when (e) {
    FocusEvent.GAIN -> PlayerAction.Play
    FocusEvent.LOSS -> PlayerAction.End
    FocusEvent.LOSS_TRANSIENT -> PlayerAction.Pause
    FocusEvent.LOSS_CAN_DUCK -> PlayerAction.Duck(0.2f)
    FocusEvent.NOISY -> PlayerAction.End
}
