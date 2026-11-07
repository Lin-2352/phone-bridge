package dev.phonebridge.app.devices

/** The kinds of request a PC can make that have an Ask / Always / Never answer. */
enum class ConsentKind(val key: String, val label: String) {
    Control("control", "control of this phone"),
    Screen("screen", "a screen share"),
    Camera("camera", "a camera request"),
    Audio("audio", "an audio share"),
}

/** Whether a "blocked by Never" note may be shown again: at most one per PC and kind a minute. */
fun shouldNotifyBlock(lastMs: Long?, nowMs: Long): Boolean = lastMs == null || nowMs < lastMs || nowMs - lastMs >= 60_000L

/** The note shown when a request was refused without a prompt because the PC is set to Never. */
fun blockedText(kind: ConsentKind, pc: String): String = "Blocked ${kind.label} from $pc (set to Never)"

/** What a request from one PC does: go ahead without asking, ask the person, or refuse without asking. */
enum class ConsentStep { AutoAllow, Ask, AutoDeny }

/**
 * The step for a request given what the person chose for that PC. Never refuses without a prompt; Always allows
 * without a prompt, except while the phone is locked, where it still asks (a request must not be answered for the
 * person while they are not there); Ask asks.
 */
fun consentStep(perm: Allow, keyguardLocked: Boolean): ConsentStep = when (perm) {
    Allow.Never -> ConsentStep.AutoDeny
    Allow.Always -> if (keyguardLocked) ConsentStep.Ask else ConsentStep.AutoAllow
    Allow.Ask -> ConsentStep.Ask
}

/** Peer ids that have a switch turned on (clipboard sync, notifications or auto-accept), for the node's per-peer filter. */
fun peersWith(all: Map<String, Any?>, peerIds: List<String>, key: String): List<String> =
    peerIds.filter { all["$it/$key"] == true }
