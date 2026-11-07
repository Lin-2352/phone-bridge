package dev.phonebridge.app.ui

/** What the lite decision looks at. */
data class LiteSignals(
    /** The manual lite switch. */
    val forced: Boolean,
    /** ActivityManager.isLowRamDevice. */
    val lowRam: Boolean,
    /** MemoryInfo.totalMem in bytes. */
    val totalMemBytes: Long,
    /** Build.VERSION.SDK_INT. */
    val sdk: Int,
    /** PowerManager.isPowerSaveMode. */
    val powerSave: Boolean,
    /** The animator duration scale; 0 means animations are off. */
    val animatorScale: Float,
    /** The frame budget tripped (FrameBudget). */
    val slowFrames: Boolean,
)

/** Why lite mode is on, in the order the checks run. */
enum class LiteReason { Forced, LowRam, SmallMemory, OldAndroid, BatterySaver, AnimationsOff, SlowFrames }

/** The first reason that turns lite on, or null when full effects are on. */
fun liteReason(s: LiteSignals): LiteReason? = when {
    s.forced -> LiteReason.Forced
    s.lowRam -> LiteReason.LowRam
    s.totalMemBytes < 3L shl 30 -> LiteReason.SmallMemory
    s.sdk < 31 -> LiteReason.OldAndroid
    s.powerSave -> LiteReason.BatterySaver
    s.animatorScale == 0f -> LiteReason.AnimationsOff
    s.slowFrames -> LiteReason.SlowFrames
    else -> null
}

/** The line under the lite switch: the reason it is on by itself, or "Off". */
fun liteText(reason: LiteReason?): String = when (reason) {
    null -> "Off"
    LiteReason.Forced -> "On: you turned it on"
    LiteReason.LowRam -> "On: this phone is low on memory"
    LiteReason.SmallMemory -> "On: this phone has little memory"
    LiteReason.OldAndroid -> "On: Android is older than 12"
    LiteReason.BatterySaver -> "On: battery saver"
    LiteReason.AnimationsOff -> "On: animations are off"
    LiteReason.SlowFrames -> "On: effects were slow on this phone"
}
