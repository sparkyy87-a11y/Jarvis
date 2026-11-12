package de.mira.app

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.net.wifi.WifiManager
import android.os.Build
import android.provider.Settings
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.mira_mobile.Device
import uniffi.mira_mobile.Mira
import uniffi.mira_mobile.MiraException
import uniffi.mira_mobile.Reply
import uniffi.mira_mobile.isPaired
import uniffi.mira_mobile.pair
import uniffi.mira_mobile.unpair
import java.util.Locale

/** Eine Zeile im Gesprächsverlauf. */
data class Entry(val fromUser: Boolean, val text: String, val ok: Boolean = true, val file: String? = null)

/**
 * Hält den Mira-Kern für die ganze App – auch wenn keine Activity offen ist
 * (dann hält [MiraService] den Prozess am Leben).
 */
class MiraApp : Application() {
    val entries = MutableStateFlow(listOf<Entry>())
    val devices = MutableStateFlow(listOf<Device>())
    /** Name des Hauptgeräts (verwaltet Kopplung, Sperren und Rechte). */
    val adminName = MutableStateFlow<String?>(null)
    val paired = MutableStateFlow(false)
    /** Hinweis, solange die Kopplung nicht abgeschlossen ist. */
    val status = MutableStateFlow<String?>(null)
    /** Zustand des Rings (Zuhören setzt die Activity selbst). */
    val orb = MutableStateFlow(OrbMode.Idle)
    /** Mikrofonpegel 0..1 während des Zuhörens. */
    val level = MutableStateFlow(0f)

    @Volatile var mira: Mira? = null
        private set
    @Volatile var inForeground = false

    val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private lateinit var tts: TextToSpeech
    @Volatile private var ttsReady = false
    private var multicastLock: WifiManager.MulticastLock? = null

    private val dataDir get() = filesDir.absolutePath

    override fun onCreate() {
        super.onCreate()
        createChannels()
        tts = TextToSpeech(this) { status ->
            if (status == TextToSpeech.SUCCESS) {
                tts.language = Locale.GERMANY
                tts.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
                    override fun onStart(utteranceId: String?) { orb.value = OrbMode.Speaking }
                    override fun onDone(utteranceId: String?) { orb.compareAndSet(OrbMode.Speaking, OrbMode.Idle) }
                    @Deprecated("Pflicht-Override älterer API")
                    override fun onError(utteranceId: String?) { orb.compareAndSet(OrbMode.Speaking, OrbMode.Idle) }
                })
                ttsReady = true
            }
        }
        if (isPaired(dataDir)) scope.launch { start() }
        scope.launch {
            while (true) {
                devices.value = mira?.devices()?.filterNot { it.thisDevice } ?: emptyList()
                status.value = mira?.status()
                adminName.value = mira?.members()?.firstOrNull { it.admin }?.name
                delay(2000)
            }
        }
    }

    /** Startet den Kern; liefert eine Fehlermeldung oder null. */
    @Synchronized
    fun start(): String? {
        if (mira != null) return null
        // Ohne Multicast-Lock verwirft Android mDNS-Pakete (Geräte finden).
        val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        multicastLock = wifi.createMulticastLock("mira").apply { setReferenceCounted(false); acquire() }
        return try {
            mira = Mira.start(dataDir, null, AndroidBridge(this))
            paired.value = true
            MiraService.start(this)
            null
        } catch (e: MiraException) {
            e.message
        }
    }

    /** Kopplung mit dem Code aus `mirad pair` (QR-Code oder eingefügt). */
    fun pairWith(code: String, name: String): String = try {
        val info = pair(dataDir, name.trim().ifEmpty { deviceName() }, "android", code.trim())
        start() ?: "Kopplung mit ${info.name} läuft …"
    } catch (e: MiraException) {
        e.message ?: "Kopplung fehlgeschlagen"
    }

    fun forget() {
        MiraService.stop(this)
        mira?.destroy()
        mira = null
        unpair(dataDir)
        paired.value = false
        entries.value = emptyList()
    }

    fun deviceName(): String =
        Settings.Global.getString(contentResolver, Settings.Global.DEVICE_NAME)?.takeIf { it.isNotBlank() }
            ?: "${Build.MANUFACTURER} ${Build.MODEL}".trim()

    fun add(entry: Entry) = entries.update { (it + entry).takeLast(200) }

    fun say(text: String) {
        if (ttsReady && text.isNotBlank()) tts.speak(text, TextToSpeech.QUEUE_FLUSH, null, "mira")
    }

    /** Text an Mira, Antwort in den Verlauf und vorlesen. */
    fun ask(text: String, speak: Boolean) {
        if (text.isBlank()) return
        add(Entry(fromUser = true, text = text))
        orb.value = OrbMode.Thinking
        scope.launch {
            val reply = withContext(Dispatchers.IO) {
                mira?.ask(text) ?: Reply(false, "Noch nicht gekoppelt.")
            }
            add(Entry(fromUser = false, text = reply.text, ok = reply.ok))
            if (!reply.ok) {
                orb.value = OrbMode.Error
                launch {
                    delay(2500)
                    orb.compareAndSet(OrbMode.Error, OrbMode.Idle)
                }
            } else if (!speak || !ttsReady) {
                orb.value = OrbMode.Idle
            }
            if (speak) say(reply.text)
        }
    }

    private fun createChannels() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_SERVICE, "Verbindung", NotificationManager.IMPORTANCE_MIN)
        )
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_EVENTS, "Dateien und Befehle", NotificationManager.IMPORTANCE_DEFAULT)
        )
    }

    companion object {
        const val CHANNEL_SERVICE = "service"
        const val CHANNEL_EVENTS = "events"
    }
}
