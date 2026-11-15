package dev.ccontrol.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

/**
 * Visible foreground service while at least one audio route is live. It keeps
 * the phone's connections (and lease renewals) alive when the UI is in the
 * background. The phone does not capture or play audio itself; the type is
 * `connectedDevice` because it controls paired computers.
 */
class RouteService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val app = application as CControlApp
        if (intent?.action == ACTION_STOP_ALL) {
            app.routes.stopAll()
            return START_NOT_STICKY
        }
        val summary = intent?.getStringExtra(EXTRA_SUMMARY) ?: "Audio route active"
        val n = notification(this, summary)
        ServiceCompat.startForeground(
            this, NOTIFICATION_ID, n,
            if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE else 0,
        )
        return START_NOT_STICKY
    }

    companion object {
        private const val CHANNEL = "routes"
        private const val NOTIFICATION_ID = 7
        private const val ACTION_STOP_ALL = "dev.ccontrol.app.STOP_ALL_ROUTES"
        private const val EXTRA_SUMMARY = "summary"

        private fun notification(ctx: Context, summary: String): Notification {
            val nm = ctx.getSystemService(NotificationManager::class.java)
            if (nm.getNotificationChannel(CHANNEL) == null) {
                nm.createNotificationChannel(NotificationChannel(CHANNEL, "Audio routes", NotificationManager.IMPORTANCE_LOW).apply {
                    description = "Shown while ccontrol is routing audio between your computers"
                })
            }
            val open = PendingIntent.getActivity(ctx, 0, Intent(ctx, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
            val stop = PendingIntent.getService(
                ctx, 1, Intent(ctx, RouteService::class.java).setAction(ACTION_STOP_ALL),
                PendingIntent.FLAG_IMMUTABLE,
            )
            return NotificationCompat.Builder(ctx, CHANNEL)
                .setSmallIcon(android.R.drawable.ic_media_play)
                .setContentTitle("Routing audio")
                .setContentText(summary)
                .setOngoing(true)
                .setContentIntent(open)
                .addAction(android.R.drawable.ic_media_pause, "Stop", stop)
                .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
                .build()
        }

        fun update(ctx: Context, summary: String) {
            ContextCompat.startForegroundService(ctx, Intent(ctx, RouteService::class.java).putExtra(EXTRA_SUMMARY, summary))
        }

        fun stop(ctx: Context) {
            ctx.stopService(Intent(ctx, RouteService::class.java))
        }
    }
}
