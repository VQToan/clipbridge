package dev.clipbridge

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

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel

class ClipboardSyncService : Service() {

    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var logcatMonitor: LogcatClipboardMonitor? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        startForegroundServiceNotification()

        val engine = SyncEngine.getInstance(this)
        engine.startSync()

        logcatMonitor = LogcatClipboardMonitor(this).also {
            it.start(serviceScope)
        }
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        super.onTaskRemoved(rootIntent)
        // Keep service alive and re-trigger foreground sync if app was swiped away
        val engine = SyncEngine.getInstance(this)
        engine.startSync()
    }

    override fun onDestroy() {
        super.onDestroy()
        logcatMonitor?.stop()
        serviceScope.cancel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun startForegroundServiceNotification() {
        val launchIntent = Intent(this, MainActivity::class.java)
        val pendingIntent = PendingIntent.getActivity(
            this, 0, launchIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        val syncIntent = Intent(this, ClipSyncTrampolineActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val syncPendingIntent = PendingIntent.getActivity(
            this, 1, syncIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("ClipBridge")
            .setContentText("Clipboard sync active")
            .setSmallIcon(android.R.drawable.ic_menu_share)
            .setContentIntent(pendingIntent)
            .addAction(android.R.drawable.ic_menu_upload, "Sync Clipboard", syncPendingIntent)
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "ClipBridge Sync Service",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Monitors and syncs clipboard across devices"
            }
            val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            manager.createNotificationChannel(channel)
        }
    }

    companion object {
        const val CHANNEL_ID = "clipbridge_sync_channel"
        const val NOTIFICATION_ID = 1001

        fun start(context: Context) {
            val intent = Intent(context, ClipboardSyncService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }
    }
}
