package dev.phonebridge.app

/** Why a phone notification is, or is not, mirrored to the PC. */
enum class MirrorVerdict { Send, BlockedByUser, BlockedByDefault, BlockedSecret }

/** Exact packages that carry codes, money or passwords. */
val DEFAULT_BLOCKED_PACKAGES: Set<String> = setOf(
    "com.google.android.apps.authenticator2", "com.authy.authy", "com.microsoft.msa.authenticator",
    "com.azure.authenticator", "org.fedorahosted.freeotp", "com.beemdevelopment.aegis", "com.twofasapp",
    "com.x8bit.bitwarden", "com.bitwarden.authenticator", "com.agilebits.onepassword", "com.lastpass.lpandroid",
    "com.google.android.apps.walletnfcrel", "com.google.android.apps.nbu.paisa.user", "com.phonepe.app",
    "net.one97.paytm", "in.org.npci.upiapp", "com.paypal.android.p2pmobile", "com.revolut.revolut",
    "com.chase.sig.android", "com.infonow.bofa", "com.wf.wellsfargomobile", "com.snapwork.hdfc",
    "com.csam.icici.bank.imobile", "com.sbi.SBIFreedomPlus", "com.axis.mobile", "com.msf.kbank.mobile",
)

private fun looksSensitive(pkg: String): Boolean = pkg.lowercase().split('.').any {
    "authenticator" in it || "banking" in it || it.startsWith("bank") || it.endsWith("bank")
}

fun blockedByDefault(pkg: String): Boolean = pkg in DEFAULT_BLOCKED_PACKAGES || looksSensitive(pkg)

/**
 * [blocked]: apps the user turned off; [allowed]: apps the user turned on although they are blocked by
 * default. An explicit block always wins; an explicit allow lifts the default and "secret" blocks.
 */
fun mirrorVerdict(pkg: String, secret: Boolean, blocked: Set<String>, allowed: Set<String>): MirrorVerdict = when {
    pkg in blocked -> MirrorVerdict.BlockedByUser
    pkg in allowed -> MirrorVerdict.Send
    blockedByDefault(pkg) -> MirrorVerdict.BlockedByDefault
    secret -> MirrorVerdict.BlockedSecret
    else -> MirrorVerdict.Send
}

data class MirrorCounts(val sent: Int = 0, val blocked: Int = 0) {
    fun after(v: MirrorVerdict) = if (v == MirrorVerdict.Send) copy(sent = sent + 1) else copy(blocked = blocked + 1)
    fun summary() = "Mirrored $sent, blocked $blocked"
}
