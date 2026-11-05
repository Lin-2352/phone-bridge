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
 * A MAC that survives a Keystore key which loads but fails in use (seen after some OEM updates as
 * "invalid key blob"): the first failure switches to [makeFallback] for good and calls [onFallback].
 */
class FallbackMac(
    private val primary: (ByteArray) -> ByteArray,
    private val makeFallback: () -> (ByteArray) -> ByteArray,
    private val onFallback: () -> Unit,
) {
    @Volatile private var fallback: ((ByteArray) -> ByteArray)? = null

    fun mac(data: ByteArray): ByteArray {
        fallback?.let { return it(data) }
        return try {
            primary(data)
        } catch (e: Exception) {
            synchronized(this) {
                fallback ?: makeFallback().also { fallback = it; onFallback() }
            }(data)
        }
    }
}

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

    private fun memoryMac(): (ByteArray) -> ByteArray {
        val k = SecretKeySpec(ByteArray(32).also { SecureRandom().nextBytes(it) }, ALG)
        return { data -> Mac.getInstance(ALG).apply { init(k) }.doFinal(data) }
    }

    private val safe = FallbackMac(
        primary = { data -> Mac.getInstance(ALG).apply { init(key()) }.doFinal(data) },
        makeFallback = ::memoryMac,
        onFallback = {
            Log.w(TAG, "CLIPSYNC Keystore key unusable, fingerprint kept in memory")
            stored = false
            runCatching { KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.deleteEntry(ALIAS) }
        },
    )

    private val mac: (ByteArray) -> ByteArray = { data -> safe.mac(data) }

    fun unchanged(ctx: Context, text: String): Boolean {
        key()
        val prefs = Prefs(ctx)
        return clipUnchanged(text, if (stored) prefs.lastClipMac else inMemory, mac)
    }

    fun remember(ctx: Context, text: String) {
        val f = clipFingerprint(text, mac)
        if (stored) Prefs(ctx).lastClipMac = f else inMemory = f
    }
}
