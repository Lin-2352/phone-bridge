package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.pb_ffi.PbException

/** M3.7 (B5): an identity that can't be unsealed means the Keystore key is gone; retrying can't fix it. */
class StartFailureTest {
    @Test
    fun a_keystore_key_that_no_longer_opens_the_identity_asks_for_a_reset() {
        for (why in listOf("javax.crypto.AEADBadTagException: tag mismatch", "sealed blob too short", "expected a 32-byte seed")) {
            assertEquals(why, StartFailure.KEYSTORE_RESET, startFailure(PbException.IdentityUnreadable(why)))
        }
    }

    @Test
    fun a_keystore_that_is_only_busy_keeps_retrying() {
        assertEquals(StartFailure.RETRY, startFailure(PbException.IdentityUnreadable("java.security.KeyStoreException: not ready")))
    }

    @Test
    fun anything_else_keeps_retrying() {
        assertEquals(StartFailure.RETRY, startFailure(RuntimeException("port busy")))
        assertEquals(StartFailure.RETRY, startFailure(PbException.Failed("AEADBadTagException")))
    }

    @Test
    fun relay_is_not_restarted_while_its_identity_is_lost() {
        assertTrue(shouldRecover(enabled = true, running = false, identityLost = false))
        assertFalse(shouldRecover(enabled = true, running = false, identityLost = true))
        assertFalse(shouldRecover(enabled = false, running = false, identityLost = false))
        assertFalse(shouldRecover(enabled = true, running = true, identityLost = false))
    }
}
