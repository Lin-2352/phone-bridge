package dev.phonebridge.app

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.pb_ffi.KeyWrapper
import uniffi.pb_ffi.PbException

/**
 * Seals the identity seed with a non-exportable AES-256-GCM key in the
 * Android Keystore. Blob format: 12-byte IV || ciphertext+tag.
 */
class KeystoreWrapper : KeyWrapper {
    private val alias = "phone_bridge_identity_v1"

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(alias, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        return gen.generateKey()
    }

    override fun wrap(plain: ByteArray): ByteArray = try {
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, key())
        c.iv + c.doFinal(plain)
    } catch (e: Exception) {
        throw PbException.Failed("keystore wrap failed: $e")
    }

    override fun unwrap(sealed: ByteArray): ByteArray = try {
        require(sealed.size > 12 + 16) { "sealed blob too short" }
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, sealed.copyOfRange(0, 12)))
        c.doFinal(sealed.copyOfRange(12, sealed.size))
    } catch (e: Exception) {
        throw PbException.Failed("keystore unwrap failed: $e")
    }
}
