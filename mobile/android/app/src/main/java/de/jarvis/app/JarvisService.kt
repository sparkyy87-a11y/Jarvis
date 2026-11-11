package de.jarvis.app

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
 * Hält Jarvis erreichbar, auch wenn die App nicht offen ist – damit der PC
 * Dateien und Befehle schicken kann. Zeigt eine dezente Dauer-Benachrichtigung.
 */
class JarvisService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = NotificationCompat.Builder(this, JarvisApp.CHANNEL_SERVICE)
            .setSmallIcon(R.drawable.ic_jarvis)
            .setContentTitle("Jarvis ist verbunden")
            .setContentText("Erreichbar für deine anderen Geräte")
            .setContentIntent(open)
            .setOngoing(true)
            .build()
        val type = if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE else 0
        ServiceCompat.startForeground(this, 1, notification, type)
        // Falls der Prozess neu gestartet wurde: Kern wieder hochfahren.
        (application as JarvisApp).start()
        return START_STICKY
    }

    companion object {
        fun start(ctx: Context) =
            ContextCompat.startForegroundService(ctx, Intent(ctx, JarvisService::class.java))

        fun stop(ctx: Context) = ctx.stopService(Intent(ctx, JarvisService::class.java))
    }
}
