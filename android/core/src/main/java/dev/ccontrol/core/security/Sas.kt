package dev.ccontrol.core.security

import java.security.MessageDigest
import java.security.SecureRandom

/** Mirrors `crates/security/src/sas.rs`; verified by the shared golden vector. */
object Sas {
    const val NONCE_LEN = 32
    private val rng = SecureRandom()

    fun randomNonce(): ByteArray = ByteArray(NONCE_LEN).also(rng::nextBytes)

    fun commitment(nonce: ByteArray): String = sha256("ccontrol-commit-v1".toByteArray(), nonce).toHex()

    fun compute(serverFp: String, clientFp: String, clientNonce: ByteArray, serverNonce: ByteArray): String {
        val d = sha256(
            "ccontrol-sas-v1".toByteArray(),
            serverFp.lowercase().toByteArray(),
            clientFp.lowercase().toByteArray(),
            clientNonce,
            serverNonce,
        )
        val n = ((d[0].toLong() and 0xff) shl 24) or ((d[1].toLong() and 0xff) shl 16) or
            ((d[2].toLong() and 0xff) shl 8) or (d[3].toLong() and 0xff)
        return "%06d".format(n % 1_000_000)
    }
}

object Fingerprint {
    /** hex(SHA-256(SubjectPublicKeyInfo DER)) — the stable device ID. */
    fun ofSpki(spkiDer: ByteArray): String = sha256(spkiDer).toHex()

    fun short(fp: String): String = fp.take(16).uppercase().chunked(4).joinToString("-")
}

internal fun sha256(vararg parts: ByteArray): ByteArray {
    val md = MessageDigest.getInstance("SHA-256")
    parts.forEach(md::update)
    return md.digest()
}

fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }

fun String.hexToBytes(): ByteArray {
    require(length % 2 == 0) { "odd hex length" }
    return ByteArray(length / 2) { i -> substring(i * 2, i * 2 + 2).toInt(16).toByte() }
}
