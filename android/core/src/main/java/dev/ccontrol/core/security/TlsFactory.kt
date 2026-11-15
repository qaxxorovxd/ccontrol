package dev.ccontrol.core.security

import java.net.Socket
import java.security.Principal
import java.security.PrivateKey
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLEngine
import javax.net.ssl.SSLSocketFactory
import javax.net.ssl.X509ExtendedKeyManager
import javax.net.ssl.X509ExtendedTrustManager

/**
 * TLS 1.3 with a keystore-backed client certificate and SPKI pinning.
 *
 * There is no CA: during pairing the server fingerprint is recorded (and, with
 * a QR code, checked against the QR value); afterwards it must match exactly.
 */
class PinningTrustManager(private val expectedFp: String?) : X509ExtendedTrustManager() {
    @Volatile
    var observedFingerprint: String? = null
        private set

    private fun check(chain: Array<out X509Certificate>?) {
        val leaf = chain?.firstOrNull() ?: throw CertificateException("no server certificate")
        val fp = Fingerprint.ofSpki(leaf.publicKey.encoded)
        observedFingerprint = fp
        if (expectedFp != null && !expectedFp.equals(fp, ignoreCase = true)) {
            throw CertificateException("computer identity changed (expected ${Fingerprint.short(expectedFp)}, got ${Fingerprint.short(fp)})")
        }
    }

    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) = check(chain)
    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?, socket: Socket?) = check(chain)
    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?, engine: SSLEngine?) = check(chain)
    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) = throw CertificateException("client role only")
    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?, socket: Socket?) = throw CertificateException("client role only")
    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?, engine: SSLEngine?) = throw CertificateException("client role only")
    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
}

private class IdentityKeyManager(private val id: DeviceIdentity) : X509ExtendedKeyManager() {
    override fun chooseClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, socket: Socket?) = id.alias
    override fun chooseEngineClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, engine: SSLEngine?) = id.alias
    override fun getClientAliases(keyType: String?, issuers: Array<out Principal>?) = arrayOf(id.alias)
    override fun getCertificateChain(alias: String?): Array<X509Certificate> = arrayOf(id.certificate)
    override fun getPrivateKey(alias: String?): PrivateKey = id.privateKey
    override fun getServerAliases(keyType: String?, issuers: Array<out Principal>?): Array<String>? = null
    override fun chooseServerAlias(keyType: String?, issuers: Array<out Principal>?, socket: Socket?): String? = null
}

object TlsFactory {
    fun socketFactory(identity: DeviceIdentity, trust: PinningTrustManager): SSLSocketFactory {
        val ctx = SSLContext.getInstance("TLS")
        ctx.init(arrayOf(IdentityKeyManager(identity)), arrayOf(trust), null)
        return ctx.socketFactory
    }
}
