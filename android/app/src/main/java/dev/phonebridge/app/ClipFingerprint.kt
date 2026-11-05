package dev.phonebridge.app

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Log
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.KeyGenerator
import javax.crypto.Mac
import javax.crypto.SecretKey
import javax.crypto.spec.SecretKeySpec

/** Lowercase hex of [mac] over the clip's UTF-8 bytes: a keyed hash, so a stored value reveals nothing. */
fun clipFingerprint(text: String, mac: (ByteArray) -> ByteArray): String =
    mac(text.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }

/** The clip matches the stored fingerprint. Anything else (none, or an old `hashCode` value) means changed. */
fun clipUnchanged(text: String, stored: String?, mac: (ByteArray) -> ByteArray): Boolean =
    stored != null && stored.length == 64 && stored == clipFingerprint(text, mac)

/**
 * The last clipboard text synced either way, kept as an HMAC-SHA256 under a key that never leaves the
 * Keystore. If the Keystore fails, a key made for this process is used and nothing is stored, so a
 * restart may re-send one unchanged clip at most.
 */
object ClipFingerprints {
    private const val TAG = "PhoneBridge"
    private const val ALIAS = "relay_clip_mac_v1"
    private const val ALG = "HmacSHA256"

    @Volatile private var key: SecretKey? = null
    @Volatile private var stored = true
    @Volatile private var inMemory: String? = null

    private fun key(): SecretKey = key ?: synchronized(this) {
        key ?: runCatching {
            val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
            (ks.getKey(ALIAS, null) as? SecretKey) ?: KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_HMAC_SHA256, "AndroidKeyStore")
                .apply { init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_SIGN).build()) }
                .generateKey()
        }.getOrElse {
            Log.w(TAG, "CLIPSYNC Keystore unavailable, fingerprint kept in memory: $it")
            stored = false
            SecretKeySpec(ByteArray(32).also { b -> SecureRandom().nextBytes(b) }, ALG)
        }.also { key = it }
    }

    private val mac: (ByteArray) -> ByteArray = { data -> Mac.getInstance(ALG).apply { init(key()) }.doFinal(data) }

    fun unchanged(ctx: Context, text: String): Boolean {
        key()
        val prefs = Prefs(ctx)
        prefs.dropLegacyClip()
        return clipUnchanged(text, if (stored) prefs.lastClipMac else inMemory, mac)
    }

    fun remember(ctx: Context, text: String) {
        val f = clipFingerprint(text, mac)
        if (stored) Prefs(ctx).lastClipMac = f else inMemory = f
    }
}
