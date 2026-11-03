package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class NotifyBlockTest {
    private fun v(pkg: String, secret: Boolean = false, blocked: Set<String> = emptySet(), allowed: Set<String> = emptySet()) =
        mirrorVerdict(pkg, secret, blocked, allowed)

    @Test fun ordinaryAppsAreSent() {
        assertEquals(MirrorVerdict.Send, v("com.whatsapp"))
        assertEquals(MirrorVerdict.Send, v("com.google.android.gm"))
        assertEquals(MirrorVerdict.Send, v("com.fun.game"))
    }

    @Test fun knownBankingAndAuthenticatorAppsAreBlockedByDefault() {
        assertEquals(MirrorVerdict.BlockedByDefault, v("com.google.android.apps.authenticator2"))
        assertEquals(MirrorVerdict.BlockedByDefault, v("com.authy.authy"))
        assertEquals(MirrorVerdict.BlockedByDefault, v("com.x8bit.bitwarden"))
        assertEquals(MirrorVerdict.BlockedByDefault, v("com.phonepe.app"))
        assertEquals(MirrorVerdict.BlockedByDefault, v("com.example.mobilebanking"))
        assertEquals(MirrorVerdict.BlockedByDefault, v("com.sbi.bank"))
        assertEquals(MirrorVerdict.BlockedByDefault, v("com.acme.authenticator.pro"))
    }

    @Test fun secretNotificationsAreNeverSentUnlessAppAllowed() {
        assertEquals(MirrorVerdict.BlockedSecret, v("com.whatsapp", secret = true))
    }

    @Test fun userBlockWins() {
        assertEquals(MirrorVerdict.BlockedByUser, v("com.whatsapp", blocked = setOf("com.whatsapp")))
        assertEquals(MirrorVerdict.BlockedByUser, v("com.whatsapp", blocked = setOf("com.whatsapp"), allowed = setOf("com.whatsapp")))
    }

    @Test fun userCanAllowADefaultBlockedApp() {
        assertEquals(MirrorVerdict.Send, v("com.phonepe.app", allowed = setOf("com.phonepe.app")))
        assertEquals(MirrorVerdict.Send, v("com.phonepe.app", secret = true, allowed = setOf("com.phonepe.app")))
    }

    @Test fun countsAddUp() {
        val c = MirrorCounts(sent = 3, blocked = 1).after(MirrorVerdict.Send).after(MirrorVerdict.BlockedByUser)
        assertEquals(MirrorCounts(4, 2), c)
        assertEquals("Mirrored 4, blocked 2", c.summary())
    }
}
