package dev.ccontrol.core.data

import android.util.AtomicFile
import dev.ccontrol.core.protocol.CapabilityManifest
import dev.ccontrol.core.protocol.Wire
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import java.io.File

/** A computer this phone has paired with. Identity = SPKI fingerprint, never IP. */
@Serializable
data class PairedDevice(
    val deviceId: String,
    val name: String,
    val customName: String? = null,
    val lastHost: String,
    val port: Int,
    val osFamily: String? = null,
    val grantedScopes: List<String> = emptyList(),
    val manifest: CapabilityManifest? = null,
    val pairedAt: Long,
    val lastSeen: Long? = null,
    val pinned: Boolean = false,
) {
    val displayName: String get() = customName?.takeIf { it.isNotBlank() } ?: name
}

/**
 * Offline-first inventory stored as one small JSON document written with
 * [AtomicFile] (crash-safe replace). Holds only non-secret metadata; keys live
 * in Android Keystore.
 */
class DeviceRepository(dir: File) {
    private val file = AtomicFile(File(dir, "devices.json"))
    private val serializer = ListSerializer(PairedDevice.serializer())
    private val _devices = MutableStateFlow(load())
    val devices: StateFlow<List<PairedDevice>> = _devices.asStateFlow()

    private fun load(): List<PairedDevice> = try {
        if (!file.baseFile.exists()) emptyList()
        else Wire.json.decodeFromString(serializer, String(file.readFully(), Charsets.UTF_8))
    } catch (e: Exception) {
        android.util.Log.e("ccontrol", "inventory unreadable; starting empty", e)
        emptyList()
    }

    @Synchronized
    private fun persist(list: List<PairedDevice>) {
        val out = file.startWrite()
        try {
            out.write(Wire.json.encodeToString(serializer, list).toByteArray(Charsets.UTF_8))
            file.finishWrite(out)
        } catch (e: Exception) {
            file.failWrite(out)
            throw e
        }
    }

    fun get(id: String): PairedDevice? = _devices.value.firstOrNull { it.deviceId == id }

    fun upsert(d: PairedDevice) = mutate { list -> list.filterNot { it.deviceId == d.deviceId } + d }

    fun update(id: String, f: (PairedDevice) -> PairedDevice) = mutate { list -> list.map { if (it.deviceId == id) f(it) else it } }

    fun remove(id: String) = mutate { list -> list.filterNot { it.deviceId == id } }

    @Synchronized
    private fun mutate(f: (List<PairedDevice>) -> List<PairedDevice>) {
        _devices.update { old -> f(old).sortedWith(compareByDescending<PairedDevice> { it.pinned }.thenBy { it.displayName.lowercase() }).also(::persist) }
    }
}
