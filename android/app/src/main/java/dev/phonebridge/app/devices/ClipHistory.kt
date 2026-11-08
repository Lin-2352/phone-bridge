package dev.phonebridge.app.devices

import java.nio.ByteBuffer
import java.security.GeneralSecurityException
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Seals and opens one record. [open] returns null when the record cannot be opened (tampered, or
 *  the key is gone); the history then reads as empty. */
interface ClipCipher {
    fun seal(plain: ByteArray, aad: ByteArray): ByteArray
    fun open(sealed: ByteArray, aad: ByteArray): ByteArray?
}

/** Where the sealed bytes live: one file in the product, memory in tests. */
interface ClipBlob {
    fun read(): ByteArray?
    fun write(bytes: ByteArray)
    fun delete()
}

/** AES-256-GCM with a key the caller supplies (the Android Keystore key in the product, a software
 *  key in tests). The Keystore requires randomised encryption, so the IV is taken from the cipher
 *  and stored with the record; the version byte and created time travel as AAD, so the time cannot
 *  be edited to dodge the expiry. Record: iv length byte, iv, ciphertext. */
class AesGcmClipCipher(private val key: () -> SecretKey?) : ClipCipher {
    override fun seal(plain: ByteArray, aad: ByteArray): ByteArray {
        val k = key() ?: throw GeneralSecurityException("no key")
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, k)
        c.updateAAD(aad)
        val ct = c.doFinal(plain)
        return byteArrayOf(c.iv.size.toByte()) + c.iv + ct
    }

    override fun open(sealed: ByteArray, aad: ByteArray): ByteArray? = try {
        val k = key() ?: return null
        val n = sealed[0].toInt()
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.DECRYPT_MODE, k, GCMParameterSpec(128, sealed, 1, n))
        c.updateAAD(aad)
        c.doFinal(sealed, 1 + n, sealed.size - 1 - n)
    } catch (e: GeneralSecurityException) {
        null
    } catch (e: RuntimeException) {
        null
    }
}

data class ClipEntry(val created: Long, val text: String)

enum class Stored { Yes, Off, Sensitive, OneTimeCode, TooLong, Empty }

/** Clipboard history on the phone: opt-in, encrypted at rest, seven days, searched in memory.
 *  A clip marked sensitive or recognised as a one-time code is never stored. */
class ClipHistory(
    private val blob: ClipBlob,
    private val cipher: ClipCipher,
    enabled: Boolean,
) {
    var enabled: Boolean = enabled
        private set

    fun setEnabled(on: Boolean) {
        enabled = on
        if (!on) blob.delete()
    }

    fun add(text: String, sensitive: Boolean, nowSecs: Long): Stored {
        if (!enabled) return Stored.Off
        if (sensitive) return Stored.Sensitive
        if (text.isBlank()) return Stored.Empty
        if (text.toByteArray().size > MAX_CLIP) return Stored.TooLong
        if (isOneTimeCode(text)) return Stored.OneTimeCode
        val all = (entries(nowSecs) + ClipEntry(nowSecs, text)).takeLast(MAX_ENTRIES)
        blob.write(encode(all))
        return Stored.Yes
    }

    /** Kept clips, oldest first. Expired and unreadable records are dropped; a file where no record
     *  opens (the key is lost) is deleted and reads as empty. */
    fun entries(nowSecs: Long): List<ClipEntry> {
        if (!enabled) return emptyList()
        val bytes = blob.read() ?: return emptyList()
        val buf = ByteBuffer.wrap(bytes)
        val out = mutableListOf<ClipEntry>()
        var total = 0
        var opened = 0
        while (buf.remaining() >= 4) {
            val n = buf.int
            if (n < 0 || n > buf.remaining()) break
            val rec = ByteArray(n).also { buf.get(it) }
            total++
            val e = decode(rec) ?: continue
            opened++
            if (nowSecs - e.created < KEEP_SECS) out += e
        }
        if (total > 0 && opened == 0) blob.delete()
        return out
    }

    /** Case-insensitive substring search, newest first. */
    fun search(query: String, nowSecs: Long): List<ClipEntry> {
        val q = query.lowercase()
        return entries(nowSecs).filter { q.isEmpty() || it.text.lowercase().contains(q) }.reversed()
    }

    private fun aad(created: Long) = ByteBuffer.allocate(9).put(VERSION).putLong(created).array()

    private fun encode(list: List<ClipEntry>): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        for (e in list) {
            val sealed = cipher.seal(e.text.toByteArray(), aad(e.created))
            val rec = ByteBuffer.allocate(8 + sealed.size).putLong(e.created).put(sealed).array()
            out.write(ByteBuffer.allocate(4).putInt(rec.size).array())
            out.write(rec)
        }
        return out.toByteArray()
    }

    private fun decode(rec: ByteArray): ClipEntry? {
        if (rec.size < 9) return null
        val created = ByteBuffer.wrap(rec, 0, 8).long
        val plain = cipher.open(rec.copyOfRange(8, rec.size), aad(created)) ?: return null
        return ClipEntry(created, String(plain))
    }

    companion object {
        const val KEEP_SECS = 7L * 24 * 3600
        const val MAX_CLIP = 64 * 1024
        const val MAX_ENTRIES = 200
        private const val VERSION: Byte = 1

        /** A short run of digits (optionally grouped by spaces or dashes) is a one-time code. */
        fun isOneTimeCode(text: String): Boolean {
            val t = text.trim()
            val digits = t.count { it.isDigit() }
            return digits in 4..10 && t.all { it.isDigit() || it == ' ' || it == '-' }
        }
    }
}
