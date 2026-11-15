package dev.ccontrol.app

import android.app.Application
import android.os.Build
import android.provider.Settings
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import dev.ccontrol.core.data.DeviceRepository
import dev.ccontrol.core.net.AppInfo
import dev.ccontrol.core.net.ConnectionManager
import dev.ccontrol.core.security.DeviceIdentity
import dev.ccontrol.core.security.KeystoreIdentity

/**
 * Manual dependency graph (Phase 1 keeps the object graph small enough that
 * Hilt adds no value yet; see docs/architecture/DECISIONS.md).
 */
class CControlApp : Application() {
    lateinit var repo: DeviceRepository
        private set
    lateinit var connections: ConnectionManager
        private set
    lateinit var appInfo: AppInfo
        private set

    @Volatile private var identityCache: DeviceIdentity? = null

    /** Keystore access is slow on some devices; create lazily off the main thread. */
    fun identity(): DeviceIdentity = identityCache ?: synchronized(this) {
        identityCache ?: KeystoreIdentity.getOrCreate().also { identityCache = it }
    }

    override fun onCreate() {
        super.onCreate()
        val name = Settings.Global.getString(contentResolver, Settings.Global.DEVICE_NAME)
            ?: "${Build.MANUFACTURER} ${Build.MODEL}"
        appInfo = AppInfo(deviceName = name, appVersion = BuildConfigVersion.NAME)
        repo = DeviceRepository(filesDir)
        connections = ConnectionManager(this, repo, ::identity, appInfo)
        ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) = connections.setForeground(true)
            override fun onStop(owner: LifecycleOwner) = connections.setForeground(false)
        })
    }
}

object BuildConfigVersion { const val NAME = "0.1.0" }
