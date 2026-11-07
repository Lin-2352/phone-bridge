package dev.phonebridge.app.devices

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
