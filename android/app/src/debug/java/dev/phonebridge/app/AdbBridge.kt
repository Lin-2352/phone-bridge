package dev.phonebridge.app

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import dev.phonebridge.app.audio.helperScript
import io.github.muntashirakon.adb.AbsAdbConnectionManager
import io.github.muntashirakon.adb.android.AdbMdns
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.File
import java.math.BigInteger
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.cert.Certificate
import java.security.cert.CertificateFactory
import java.security.spec.PKCS8EncodedKeySpec
import java.util.Date
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import org.bouncycastle.asn1.x500.X500Name
import org.bouncycastle.cert.jcajce.JcaX509v3CertificateBuilder
import org.bouncycastle.operator.jcajce.JcaContentSignerBuilder

private const val WRAP_ALIAS = "relay_adb_identity"
private const val IDENTITY_FILE = "adb_identity"
private const val KEY_BITS = 2048
private const val GCM_TAG_BITS = 128
private const val CONNECT_FIND_MS = 5000L
private const val STREAM_TIMEOUT_S = 10L
private const val SHELL_LIMIT_MS = 30_000L
private const val READ_BYTES = 1024
private const val CERT_YEARS_MS = 20L * 365 * 24 * 60 * 60 * 1000

/**
 * Relay talking to this phone's own adb daemon. The identity it pairs with amounts to shell access
 * to this phone, so it stays in app-private storage outside backups, encrypted under a key that
 * lives in Android's keystore, and only [HelperStarter]'s fixed commands ever use it.
 */
object AdbBridge {
    private class Manager(private val key: PrivateKey, private val cert: Certificate) : AbsAdbConnectionManager() {
        init {
            setApi(Build.VERSION.SDK_INT)
            setTimeout(STREAM_TIMEOUT_S, TimeUnit.SECONDS)
            setThrowOnUnauthorised(true)
        }

        override fun getPrivateKey() = key
        override fun getCertificate() = cert
        override fun getDeviceName() = APP_NAME
    }

    private fun file(ctx: Context) = File(ctx.noBackupFilesDir, IDENTITY_FILE)

    private fun wrapKey(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(WRAP_ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(WRAP_ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build(),
        )
        return gen.generateKey()
    }

    private fun newIdentity(ctx: Context): Pair<PrivateKey, Certificate> {
        val pair = KeyPairGenerator.getInstance("RSA").apply { initialize(KEY_BITS, SecureRandom()) }.generateKeyPair()
        val now = System.currentTimeMillis()
        val name = X500Name("CN=$APP_NAME")
        val holder = JcaX509v3CertificateBuilder(
            name, BigInteger(64, SecureRandom()), Date(now), Date(now + CERT_YEARS_MS), name, pair.public,
        ).build(JcaContentSignerBuilder("SHA256withRSA").build(pair.private))
        val cert = CertificateFactory.getInstance("X.509").generateCertificate(ByteArrayInputStream(holder.encoded))

        val plain = ByteArrayOutputStream()
        DataOutputStream(plain).use { o ->
            val k = pair.private.encoded
            o.writeInt(k.size)
            o.write(k)
            o.write(cert.encoded)
        }
        val c = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, wrapKey()) }
        val sealed = c.doFinal(plain.toByteArray())
        DataOutputStream(file(ctx).outputStream()).use { o ->
            o.writeInt(c.iv.size)
            o.write(c.iv)
            o.write(sealed)
        }
        return pair.private to cert
    }

    private fun readIdentity(ctx: Context): Pair<PrivateKey, Certificate>? {
        val f = file(ctx)
        if (!f.exists()) return null
        val all = DataInputStream(f.inputStream()).use { i ->
            val iv = ByteArray(i.readInt()).also { i.readFully(it) }
            val c = Cipher.getInstance("AES/GCM/NoPadding")
            c.init(Cipher.DECRYPT_MODE, wrapKey(), GCMParameterSpec(GCM_TAG_BITS, iv))
            c.doFinal(i.readBytes())
        }
        DataInputStream(all.inputStream()).use { i ->
            val raw = ByteArray(i.readInt()).also { i.readFully(it) }
            val key = KeyFactory.getInstance("RSA").generatePrivate(PKCS8EncodedKeySpec(raw))
            return key to CertificateFactory.getInstance("X.509").generateCertificate(i)
        }
    }

    private fun manager(ctx: Context): Manager {
        val (k, c) = readIdentity(ctx) ?: newIdentity(ctx)
        return Manager(k, c)
    }

    /** Finds Android's "Pair with code" port while that dialog is open; returns how to stop looking. */
    fun findPairingPort(ctx: Context, found: (Int) -> Unit): () -> Unit {
        val m = AdbMdns(ctx.applicationContext, AdbMdns.SERVICE_TYPE_TLS_PAIRING) { host, port ->
            if (host != null && port > 0) found(port)
        }
        m.start()
        return { m.stop() }
    }

    /** Throws when the code is wrong or the dialog was closed. */
    fun pair(ctx: Context, port: Int, code: String) {
        val m = manager(ctx)
        try {
            m.pair("127.0.0.1", port, code)
        } finally {
            runCatching { m.close() }
        }
    }

    fun hasIdentity(ctx: Context) = file(ctx).exists()

    /**
     * Connects to this phone's Wireless debugging and runs [commands] in one shell, waiting for it to
     * exit. They go in through the stream because adbd logs a command in the service name, and one
     * holds the helper's secret. Throws when not connectable or not paired. Returns what the shell
     * printed, errors included, and callers check the result itself: a refused command still exits.
     */
    fun shell(ctx: Context, commands: List<String>): String {
        val m = manager(ctx)
        val giveUp = Thread({
            try {
                Thread.sleep(SHELL_LIMIT_MS)
                runCatching { m.close() }
            } catch (_: InterruptedException) {
                // Finished in time.
            }
        }, "pb-adb-limit").apply {
            isDaemon = true
            start()
        }
        try {
            check(m.connectTls(ctx.applicationContext, CONNECT_FIND_MS)) { "adb connection was not made" }
            return m.openStream("exec:sh").use { s ->
                s.openOutputStream().run {
                    write(helperScript(listOf("exec 2>&1") + commands).toByteArray())
                    flush()
                }
                val input = s.openInputStream()
                val buf = ByteArray(READ_BYTES)
                val printed = ByteArrayOutputStream()
                try {
                    while (true) {
                        val n = input.read(buf)
                        if (n == -1) break
                        printed.write(buf, 0, n)
                    }
                } catch (e: java.io.IOException) {
                    // The library throws this when the far end closes during a read: that's the shell exiting.
                    if (!s.isClosed) throw e
                }
                printed.toString()
            }
        } finally {
            giveUp.interrupt()
            runCatching { m.close() }
        }
    }

    /** Relay can't reach this phone's adb after this, whatever the phone's own paired list shows. */
    fun forget(ctx: Context) {
        runCatching { file(ctx).delete() }
        runCatching { KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.deleteEntry(WRAP_ALIAS) }
    }
}
