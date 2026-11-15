package dev.ccontrol.core.net

import java.net.URLDecoder

/** `ccontrol://pair?h=192.168.1.14&p=47800&fp=<64 hex>&n=Name` shown by the agent's settings page. */
data class PairingUri(val host: String, val port: Int, val fingerprint: String, val name: String?) {
    companion object {
        private val HEX64 = Regex("^[0-9a-fA-F]{64}$")
        private val HOST = Regex("^[0-9A-Za-z.:\\-\\[\\]%]{1,253}$")

        fun parse(raw: String): PairingUri? {
            val s = raw.trim()
            if (!s.startsWith("ccontrol://pair?")) return null
            val params = s.substringAfter('?').split('&').mapNotNull {
                val k = it.substringBefore('=', "")
                if (k.isEmpty()) null else k to URLDecoder.decode(it.substringAfter('='), "UTF-8")
            }.toMap()
            val host = params["h"]?.takeIf { HOST.matches(it) } ?: return null
            val port = params["p"]?.toIntOrNull()?.takeIf { it in 1..65535 } ?: return null
            val fp = params["fp"]?.takeIf { HEX64.matches(it) }?.lowercase() ?: return null
            return PairingUri(host, port, fp, params["n"]?.take(64))
        }
    }
}
