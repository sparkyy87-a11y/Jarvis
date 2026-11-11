package de.jarvis.app

import android.app.NotificationManager
import android.app.SearchManager
import android.content.ActivityNotFoundException
import android.media.AudioManager
import android.net.Uri
import android.provider.MediaStore
import android.view.KeyEvent
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
            ?: return Reply(false, "$label lässt sich nicht öffnen.")
        return launch(intent, label, "$label wird geöffnet.")
    }

    override fun openUrl(url: String): Reply =
        launch(Intent(Intent.ACTION_VIEW, Uri.parse(url)), "Suche", "Ich öffne die Suche.")

    override fun playMusic(query: String, service: String): Reply {
        val term = query.replace(" von ", " ")
        val (pkg, name, web) = when (service) {
            "spotify" -> Triple("com.spotify.music", "Spotify", "https://open.spotify.com/search/" + Uri.encode(term))
            "youtube-music" -> Triple("com.google.android.apps.youtube.music", "YouTube Music", "https://music.youtube.com/search?q=" + Uri.encode(term))
            else -> Triple("com.apple.android.music", "Apple Music", "https://music.apple.com/search?term=" + Uri.encode(term))
        }
        // „Spiele per Suche": Apple Music und Spotify spielen den besten Treffer direkt.
        val play = Intent(MediaStore.INTENT_ACTION_MEDIA_PLAY_FROM_SEARCH)
            .setPackage(pkg)
            .putExtra(MediaStore.EXTRA_MEDIA_FOCUS, "vnd.android.cursor.item/*")
            .putExtra(SearchManager.QUERY, term)
        if (play.resolveActivity(app.packageManager) != null) {
            return launch(play, name, "Ich spiele „$query“ mit $name.")
        }
        return launch(Intent(Intent.ACTION_VIEW, Uri.parse(web)), name, "$name zeigt „$query“.")
    }

    override fun media(command: String): Reply {
        val (code, text) = when (command) {
            "play" -> KeyEvent.KEYCODE_MEDIA_PLAY to "Weiter geht's."
            "pause" -> KeyEvent.KEYCODE_MEDIA_PAUSE to "Pausiert."
            "next" -> KeyEvent.KEYCODE_MEDIA_NEXT to "Nächster Titel."
            else -> KeyEvent.KEYCODE_MEDIA_PREVIOUS to "Vorheriger Titel."
        }
        // Medientasten wirken auf den Player, der gerade läuft – auch im Hintergrund.
        val audio = app.getSystemService(AudioManager::class.java)
        audio.dispatchMediaKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, code))
        audio.dispatchMediaKeyEvent(KeyEvent(KeyEvent.ACTION_UP, code))
        return Reply(true, text)
    }

    /** Startet eine Activity; im Hintergrund verbietet Android das – dann als Benachrichtigung. */
    private fun launch(intent: Intent, label: String, okText: String): Reply {
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        return if (app.inForeground) {
            try {
                app.startActivity(intent)
                Reply(true, okText)
            } catch (e: ActivityNotFoundException) {
                Reply(false, "Dafür ist auf dem Handy keine App installiert.")
            }
        } else {
            notify("$label öffnen", "Antippen: $okText", intent)
            Reply(true, "Auf dem Handy wartet eine Benachrichtigung ($label).")
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
