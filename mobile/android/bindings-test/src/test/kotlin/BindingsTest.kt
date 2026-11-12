import uniffi.mira_mobile.Mira
import uniffi.mira_mobile.MiraException
import uniffi.mira_mobile.PlatformBridge
import uniffi.mira_mobile.Reply
import uniffi.mira_mobile.isPaired
import uniffi.mira_mobile.pair
import uniffi.mira_mobile.parsePairingCode
import java.io.File
import java.nio.file.Files
import java.util.Collections
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class FakeBridge : PlatformBridge {
    val opened: MutableList<String> = Collections.synchronizedList(mutableListOf())
    val received: MutableList<String> = Collections.synchronizedList(mutableListOf())
    override fun openApp(app: String): Reply {
        opened += app
        return Reply(true, "$app wird geöffnet.")
    }
    override fun speak(text: String) {}
    override fun fileReceived(path: String) { received += path }
    override fun openUrl(url: String): Reply { opened += url; return Reply(true, "geöffnet") }
    override fun playMusic(query: String, service: String): Reply { opened += "$service:$query"; return Reply(true, "$query läuft") }
    override fun media(command: String): Reply = Reply(true, command)
}

class BindingsTest {
    private val bin = File(System.getProperty("mira.bin"))

    @Test
    fun errorsArriveAsKotlinExceptions() {
        val e = assertFailsWith<MiraException> { parsePairingCode("https://example.com") }
        assertTrue(e.message!!.contains("kein Mira-Kopplungscode"), e.message)
        val dir = Files.createTempDirectory("phone").toString()
        assertFalse(isPaired(dir))
        assertFailsWith<MiraException> { Mira.start(dir, null, FakeBridge()) }
    }

    @Test
    fun phonePairsWithRealPcAndExchangesFiles() {
        // Echter PC: mirad mit eigenem HOME und Port.
        val pcHome = Files.createTempDirectory("pc").toFile()
        val cfg = File(pcHome, "mira.toml")
        fun mirad(vararg args: String): ProcessBuilder =
            ProcessBuilder(listOf(File(bin, "mirad").path) + args).apply {
                environment()["HOME"] = pcHome.path
                environment()["MIRA_CONFIG"] = cfg.path
                environment()["ANTHROPIC_API_KEY"] = ""
                environment()["RUST_BACKTRACE"] = "0"
            }
        assertEquals(0, mirad("init", "--name", "Gaming-PC").start().waitFor())
        cfg.writeText(
            cfg.readText()
                .replace("port = 7879", "port = 7979")
                .replace("control_port = 7878", "control_port = 7978")
                .replace("mdns = true", "mdns = false")
                .replace("provider = \"auto\"", "provider = \"off\""),
        )
        val pc = mirad().redirectErrorStream(true).redirectOutput(File(pcHome, "pc.log")).start()
        try {
            val tokenFile = File(pcHome, "control.token")
            val ready = System.currentTimeMillis() + 10_000
            while (!tokenFile.exists()) { check(System.currentTimeMillis() < ready) { "mirad startet nicht" }; Thread.sleep(100) }
            Thread.sleep(300)
            // Kopplungslink mit Einmal-Code genau so, wie `mirad pair` ihn zeigt.
            val pairOut = mirad("pair").redirectErrorStream(true).start().inputStream.bufferedReader().readText()
            val link = Regex("mira://pair\\?[^\"\\s]*").find(pairOut)?.value ?: error("Kein Link:\n$pairOut")
            val phoneDir = Files.createTempDirectory("phone").toString()
            val info = pair(phoneDir, "Pixel", "android", link)
            assertEquals("Gaming-PC", info.name)
            // mDNS im Test aus, damit nichts ins Netz geht.
            val phoneCfg = File(phoneDir, "mira.toml")
            phoneCfg.writeText(phoneCfg.readText().replace("mdns = true", "mdns = false"))

            val bridge = FakeBridge()
            val phone = Mira.start(phoneDir, null, bridge)
            assertTrue(phone.status()?.contains("Kopplung läuft") ?: true)
            val until = System.currentTimeMillis() + 15_000
            while (phone.devices().none { it.name == "Gaming-PC" }) {
                check(System.currentTimeMillis() < until) { "Handy findet den PC nicht:\n" + File(pcHome, "pc.log").readText() }
                Thread.sleep(100)
            }

            // Rust ruft Kotlin zurück (PlatformBridge.openApp)
            val r = phone.ask("öffne Spotify")
            assertTrue(r.ok, r.text)
            assertEquals(null, phone.status(), "Kopplung abgeschlossen")
            val admin = phone.members().first()
            assertTrue(admin.admin && admin.name == "Gaming-PC", "Hauptgerät: $admin")
            assertFalse(phone.members().single { it.thisDevice }.controlMain, "Hauptgerät nur nach Freigabe steuerbar")
            assertEquals(listOf("Spotify"), bridge.opened.toList())

            // Google-Suche und Musik laufen über die Kotlin-Brücke
            assertTrue(phone.ask("google Wetter Berlin").ok)
            val music = phone.ask("spiel Musik von Adele")
            assertEquals("Adele läuft", music.text)
            assertEquals(
                listOf("Spotify", "https://www.google.com/search?q=Wetter+Berlin", "apple-music:Adele"),
                bridge.opened.toList(),
            )

            // Datei Handy -> PC
            val photo = File(phoneDir, "urlaub.jpg").apply { writeBytes(ByteArray(200_000) { (it % 251).toByte() }) }
            val sent = phone.sendFile(photo.path, "pc", "Bilder")
            assertTrue(sent.ok, sent.text)
            val atPc = File(pcHome, "Pictures/urlaub.jpg")
            assertTrue(atPc.readBytes().contentEquals(photo.readBytes()))

            // Datei PC -> Handy, per Sprachbefehl am PC über die lokale Steuerung
            File(pcHome, "Documents").mkdirs()
            File(pcHome, "Documents/Bericht.pdf").writeText("Quartalsbericht")
            val ask = ProcessBuilder(File(bin, "mira").path, "sende", "Datei", "Bericht", "an", "das", "Handy").apply {
                environment()["HOME"] = pcHome.path
                environment()["MIRA_CONFIG"] = cfg.path
            }.redirectErrorStream(true).start()
            val answer = ask.inputStream.bufferedReader().readText()
            assertEquals(0, ask.waitFor(), answer)
            assertTrue(answer.contains("angekommen"), answer)
            val got = File(phone.filesDir(), "Downloads/Bericht.pdf")
            assertEquals("Quartalsbericht", got.readText())
            assertTrue(bridge.received.any { it.endsWith("Bericht.pdf") })
            phone.destroy()
        } finally {
            pc.destroy()
        }
    }
}
