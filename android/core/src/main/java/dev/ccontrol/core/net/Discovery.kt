package dev.ccontrol.core.net

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.util.Log
import dev.ccontrol.core.protocol.ProtocolInfo
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import java.net.Inet4Address

/** An agent seen on the LAN. Unauthenticated hint: identity is verified by TLS. */
data class DiscoveredAgent(
    val deviceId: String,
    val name: String,
    val host: String,
    val port: Int,
    val osFamily: String?,
    val protocolVersion: Int?,
)

/** DNS-SD browse for `_ccontrol._tcp` via [NsdManager] (no multicast lock needed). */
class Discovery(context: Context) {
    private val nsd = context.getSystemService(NsdManager::class.java)

    fun browse(): Flow<List<DiscoveredAgent>> = callbackFlow {
        val found = LinkedHashMap<String, DiscoveredAgent>()
        val resolveQueue = ArrayDeque<NsdServiceInfo>()
        var resolving = false

        fun publish() { trySend(found.values.toList()) }

        fun onResolved(info: NsdServiceInfo) {
            val attrs = info.attributes.mapValues { (_, v) -> v?.toString(Charsets.UTF_8) }
            val id = attrs["id"] ?: return
            val host = hostOf(info) ?: return
            found[id] = DiscoveredAgent(
                deviceId = id,
                name = attrs["name"] ?: info.serviceName,
                host = host,
                port = info.port,
                osFamily = attrs["os"],
                protocolVersion = attrs["v"]?.toIntOrNull(),
            )
            publish()
        }

        lateinit var resolveNext: () -> Unit
        resolveNext = {
            synchronized(resolveQueue) {
                if (!resolving) {
                    resolveQueue.removeFirstOrNull()?.let { info ->
                        resolving = true
                        @Suppress("DEPRECATION")
                        nsd.resolveService(info, object : NsdManager.ResolveListener {
                            override fun onResolveFailed(si: NsdServiceInfo, errorCode: Int) {
                                synchronized(resolveQueue) { resolving = false }
                                resolveNext()
                            }
                            override fun onServiceResolved(si: NsdServiceInfo) {
                                onResolved(si)
                                synchronized(resolveQueue) { resolving = false }
                                resolveNext()
                            }
                        })
                    }
                }
            }
        }

        val listener = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) {}
            override fun onDiscoveryStopped(serviceType: String) {}
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                Log.w("ccontrol", "mDNS discovery failed: $errorCode")
                publish()
            }
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
            override fun onServiceFound(info: NsdServiceInfo) {
                synchronized(resolveQueue) { resolveQueue.addLast(info) }
                resolveNext()
            }
            override fun onServiceLost(info: NsdServiceInfo) {
                val removed = found.entries.removeIf { it.value.name == info.serviceName || info.serviceName.endsWith(it.key.take(8)) }
                if (removed) publish()
            }
        }
        publish()
        nsd.discoverServices(ProtocolInfo.MDNS_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
        awaitClose {
            runCatching { nsd.stopServiceDiscovery(listener) }
        }
    }

    private fun hostOf(info: NsdServiceInfo): String? {
        val addrs = if (Build.VERSION.SDK_INT >= 34) info.hostAddresses else listOfNotNull(@Suppress("DEPRECATION") info.host)
        val pick = addrs.firstOrNull { it is Inet4Address } ?: addrs.firstOrNull()
        return pick?.hostAddress
    }
}
