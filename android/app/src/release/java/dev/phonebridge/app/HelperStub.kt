package dev.phonebridge.app

import android.content.Context
import dev.phonebridge.app.audio.AudioReason

// Release and qa builds carry no audio helper (and none of its adb, Conscrypt and Bouncy Castle
// libraries). These stand-ins keep the shared code's calls compiling; none of them can start anything.

/** This build carries no audio helper. */
const val HELPER_BUILT = false

object HelperStarter {
    fun ensure(ctx: Context, asked: Boolean = false): Boolean = false

    fun maintain(ctx: Context) = Unit
}

object HelperAudio {
    fun alive(ctx: Context): Boolean = false
}

/** Never constructed: [HelperAudio.alive] is always false here. */
class HelperCapture(
    ctx: Context,
    sampleRate: Int,
    channels: Int,
    onEnded: (AudioReason?) -> Unit,
    onFrame: (ByteArray, Int) -> Unit,
) : AudioCapture {
    override fun start(): AudioStartResult = AudioStartResult(AudioReason.UNAVAILABLE)

    override fun stop() = Unit

    override fun takeMaxFrameGapMs(): Long = 0
}
