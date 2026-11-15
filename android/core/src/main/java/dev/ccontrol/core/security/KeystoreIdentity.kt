package dev.ccontrol.core.security

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.math.BigInteger
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.cert.X509Certificate
import java.security.spec.ECGenParameterSpec
import java.util.Calendar
import javax.security.auth.x500.X500Principal

/** Long-lived phone identity. The private key never leaves Android Keystore. */
class DeviceIdentity(
    val alias: String,
    val privateKey: PrivateKey,
    val certificate: X509Certificate,
) {
    val fingerprint: String = Fingerprint.ofSpki(certificate.publicKey.encoded)
}

object KeystoreIdentity {
    private const val ALIAS = "ccontrol_device_identity_v1"
    private const val PROVIDER = "AndroidKeyStore"

    @Synchronized
    fun getOrCreate(): DeviceIdentity {
        val ks = KeyStore.getInstance(PROVIDER).apply { load(null) }
        if (!ks.containsAlias(ALIAS)) generate()
        val key = ks.getKey(ALIAS, null) as PrivateKey
        val cert = ks.getCertificate(ALIAS) as X509Certificate
        return DeviceIdentity(ALIAS, key, cert)
    }

    /** Deletes the identity. Every computer will then see this phone as a new, unpaired device. */
    @Synchronized
    fun reset() {
        val ks = KeyStore.getInstance(PROVIDER).apply { load(null) }
        if (ks.containsAlias(ALIAS)) ks.deleteEntry(ALIAS)
    }

    private fun generate() {
        val start = Calendar.getInstance().apply { add(Calendar.DAY_OF_YEAR, -1) }.time
        val end = Calendar.getInstance().apply { add(Calendar.YEAR, 50) }.time
        val spec = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            // NONE is required for TLS client authentication through Conscrypt.
            .setDigests(KeyProperties.DIGEST_NONE, KeyProperties.DIGEST_SHA256, KeyProperties.DIGEST_SHA384, KeyProperties.DIGEST_SHA512)
            .setCertificateSubject(X500Principal("CN=ccontrol phone"))
            .setCertificateSerialNumber(BigInteger.ONE)
            .setCertificateNotBefore(start)
            .setCertificateNotAfter(end)
            .build()
        KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, PROVIDER).apply {
            initialize(spec)
            generateKeyPair()
        }
    }
}
