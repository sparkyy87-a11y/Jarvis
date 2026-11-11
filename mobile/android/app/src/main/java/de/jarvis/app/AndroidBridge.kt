package de.jarvis.app

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.content.pm.PackageManager
import androidx.core.app.NotificationCompat
import uniffi.jarvis_mobile.PlatformBridge
import uniffi.jarvis_mobile.Reply
import java.io.File

/** Was Jarvis auf Android selbst tun kann – aufgerufen vom Rust-Kern. */
class AndroidBridge(private val app: JarvisApp) : PlatformBridge {

    override fun openApp(app: String): Reply {
        val pm = this.app.packageManager
        val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        val wanted = squash(app)
        if (wanted.isEmpty()) return Reply(false, "Welche App soll ich öffnen?")
        val best = pm.queryIntentActivities(launcher, PackageManager.MATCH_ALL)
            .map { it to it.loadLabel(pm).toString() }
            .map { (info, label) -> Triple(info, label, squash(label)) }
            .filter { (_, _, s) -> s.contains(wanted) || (s.length >= 3 && wanted.contains(s)) }
            .minByOrNull { (_, _, s) -> if (s == wanted) 0 else s.length }
            ?: return Reply(false, "Auf diesem Handy gibt es keine App „$app“.")
        val (info, label) = best
        val intent = pm.getLaunchIntentForPackage(info.activityInfo.packageName)
            ?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            ?: return Reply(false, "$label lässt sich nicht öffnen.")
        return if (this.app.inForeground) {
            this.app.startActivity(intent)
            Reply(true, "$label wird geöffnet.")
        } else {
            // Android verbietet Apps im Hintergrund, andere Apps zu starten:
            // stattdessen eine Benachrichtigung zum Antippen.
            notify("$label öffnen", "Antippen, um $label zu starten.", intent)
            Reply(true, "Auf dem Handy wartet eine Benachrichtigung zum Öffnen von $label.")
        }
    }

    override fun speak(text: String) = app.say(text)

    override fun fileReceived(path: String) {
        val name = File(path).name
        app.add(Entry(fromUser = false, text = "Datei empfangen: $name", file = path))
        val open = Intent(app, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        notify("Datei empfangen", name, open)
    }

    private fun notify(title: String, text: String, intent: Intent) {
        val pending = PendingIntent.getActivity(
            app, title.hashCode(), intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = NotificationCompat.Builder(app, JarvisApp.CHANNEL_EVENTS)
            .setSmallIcon(R.drawable.ic_jarvis)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(pending)
            .setAutoCancel(true)
            .build()
        app.getSystemService(NotificationManager::class.java).notify(title.hashCode(), n)
    }

    private fun squash(s: String) = s.lowercase().filter { it.isLetterOrDigit() }
}
