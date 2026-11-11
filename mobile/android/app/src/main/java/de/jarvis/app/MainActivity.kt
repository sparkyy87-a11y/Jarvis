package de.jarvis.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.webkit.MimeTypeMap
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.AttachFile
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

class MainActivity : ComponentActivity() {
    private val app get() = application as JarvisApp

    /** Datei, die gerade an ein Gerät geschickt werden soll (aus „Teilen" oder Auswahl). */
    private val pendingShare = MutableStateFlow<Uri?>(null)
    private val pairMessage = MutableStateFlow<String?>(null)
    private val listening = MutableStateFlow(false)
    private var recognizer: SpeechRecognizer? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        handleIntent(intent)
        setContent { JarvisTheme { Screen() } }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        app.inForeground = true
    }

    override fun onPause() {
        super.onPause()
        app.inForeground = false
    }

    override fun onDestroy() {
        recognizer?.destroy()
        super.onDestroy()
    }

    private fun handleIntent(intent: Intent?) {
        when (intent?.action) {
            Intent.ACTION_VIEW -> intent.data?.takeIf { it.scheme == "jarvis" }?.let { uri ->
                app.scope.launch { pairMessage.value = app.pairWith(uri.toString()) }
            }
            Intent.ACTION_SEND -> {
                val uri = if (Build.VERSION.SDK_INT >= 33) {
                    intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
                } else {
                    @Suppress("DEPRECATION") intent.getParcelableExtra(Intent.EXTRA_STREAM)
                }
                pendingShare.value = uri
            }
        }
    }

    // ---------------------------------------------------------------- Sprache

    private fun startListening() {
        if (!SpeechRecognizer.isRecognitionAvailable(this)) {
            app.add(Entry(false, "Auf diesem Handy ist keine Spracherkennung verfügbar.", ok = false))
            return
        }
        val r = recognizer ?: SpeechRecognizer.createSpeechRecognizer(this).also { recognizer = it }
        r.setRecognitionListener(object : RecognitionListener {
            override fun onResults(results: Bundle) {
                listening.value = false
                val text = results.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()
                if (!text.isNullOrBlank()) app.ask(text, speak = true)
            }
            override fun onError(error: Int) {
                listening.value = false
                if (error != SpeechRecognizer.ERROR_NO_MATCH && error != SpeechRecognizer.ERROR_SPEECH_TIMEOUT) {
                    app.add(Entry(false, "Spracherkennung fehlgeschlagen (Fehler $error).", ok = false))
                }
            }
            override fun onReadyForSpeech(params: Bundle?) { listening.value = true }
            override fun onEndOfSpeech() { listening.value = false }
            override fun onBeginningOfSpeech() {}
            override fun onRmsChanged(rmsdB: Float) {
                // Android liefert etwa -2 … 10 dB
                app.level.value = ((rmsdB + 2f) / 12f).coerceIn(0f, 1f)
            }
            override fun onBufferReceived(buffer: ByteArray?) {}
            override fun onPartialResults(partialResults: Bundle?) {}
            override fun onEvent(eventType: Int, params: Bundle?) {}
        })
        val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
            .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            .putExtra(RecognizerIntent.EXTRA_LANGUAGE, "de-DE")
            .putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true)
        r.startListening(intent)
    }

    // ---------------------------------------------------------------- Dateien

    /** Kopiert die Datei in den Cache (Android gibt nur content://-Adressen her) und schickt sie. */
    private suspend fun sendFile(uri: Uri, device: String) {
        val name = displayName(uri) ?: "datei"
        app.add(Entry(true, "„$name“ an $device senden"))
        val reply = withContext(Dispatchers.IO) {
            val dir = File(cacheDir, "senden").apply { mkdirs() }
            val copy = File(dir, name)
            contentResolver.openInputStream(uri)?.use { input ->
                copy.outputStream().use { input.copyTo(it) }
            }
            try {
                app.jarvis?.sendFile(copy.absolutePath, device, null)
            } finally {
                copy.delete()
            }
        }
        app.add(Entry(false, reply?.text ?: "Noch nicht gekoppelt.", ok = reply?.ok ?: false))
    }

    private fun displayName(uri: Uri): String? =
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        } ?: uri.lastPathSegment

    private fun openFile(path: String) {
        val file = File(path)
        val uri = FileProvider.getUriForFile(this, "$packageName.files", file)
        val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "*/*"
        val view = Intent(Intent.ACTION_VIEW).setDataAndType(uri, mime).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        startActivity(Intent.createChooser(view, file.name))
    }

    // ---------------------------------------------------------------- Oberfläche

    @Composable
    private fun Screen() {
        val paired by app.paired.collectAsState()
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            Box(Modifier.safeDrawingPadding().imePadding()) {
                if (paired) Chat() else Pairing()
            }
        }
    }

    @Composable
    private fun Pairing() {
        var code by remember { mutableStateOf("") }
        val message by pairMessage.collectAsState()
        val scope = androidx.compose.runtime.rememberCoroutineScope()
        Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.Center) {
            Text("Jarvis", style = MaterialTheme.typography.displaySmall, fontWeight = FontWeight.Bold)
            Spacer(Modifier.height(16.dp))
            Text(
                "Mit deinem PC koppeln:\n\n" +
                    "1. Am PC im Terminal: jarvisd pair\n" +
                    "2. Den QR-Code mit der Handykamera scannen – Jarvis öffnet sich und ist gekoppelt.\n\n" +
                    "Oder den Code hier einfügen:",
                style = MaterialTheme.typography.bodyLarge,
            )
            Spacer(Modifier.height(16.dp))
            OutlinedTextField(
                value = code, onValueChange = { code = it },
                label = { Text("jarvis://pair?…") }, singleLine = true, modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(12.dp))
            Button(
                onClick = { scope.launch { pairMessage.value = withContext(Dispatchers.IO) { app.pairWith(code) } } },
                enabled = code.isNotBlank(),
            ) { Text("Koppeln") }
            message?.let {
                Spacer(Modifier.height(12.dp))
                Text(it, color = MaterialTheme.colorScheme.error)
            }
        }
    }

    @Composable
    private fun Chat() {
        val entries by app.entries.collectAsState()
        val devices by app.devices.collectAsState()
        val isListening by listening.collectAsState()
        val orbState by app.orb.collectAsState()
        val level by app.level.collectAsState()
        val share by pendingShare.collectAsState()
        var input by remember { mutableStateOf("") }
        var menu by remember { mutableStateOf(false) }
        val listState = rememberLazyListState()
        val scope = androidx.compose.runtime.rememberCoroutineScope()
        val context = LocalContext.current

        val micPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
            if (ok) startListening()
        }
        val notifyPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {}
        val pickFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            pendingShare.value = uri
        }
        LaunchedEffect(Unit) {
            if (Build.VERSION.SDK_INT >= 33 &&
                ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
            ) notifyPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        LaunchedEffect(entries.size) { if (entries.isNotEmpty()) listState.animateScrollToItem(entries.size - 1) }

        Column(Modifier.fillMaxSize()) {
            Row(Modifier.fillMaxWidth().padding(start = 16.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Jarvis", style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold)
                    Text(app.jarvis?.deviceName() ?: "", style = MaterialTheme.typography.bodySmall)
                }
                Box {
                    IconButton(onClick = { menu = true }) { Icon(Icons.Filled.MoreVert, "Menü") }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        DropdownMenuItem(text = { Text("Kopplung aufheben") }, onClick = { menu = false; app.forget() })
                    }
                }
            }
            Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp)) {
                if (devices.isEmpty()) {
                    AssistChip(onClick = {}, label = { Text("Suche deine Geräte …") })
                }
                devices.forEach { d ->
                    AssistChip(onClick = {}, label = { Text(d.name) }, modifier = Modifier.padding(end = 8.dp))
                }
            }
            Box(Modifier.fillMaxWidth().padding(vertical = 8.dp), contentAlignment = Alignment.Center) {
                JarvisOrb(
                    mode = if (isListening) OrbMode.Listening else orbState,
                    level = level,
                    size = if (entries.isEmpty()) 260.dp else 170.dp,
                    onClick = {
                        if (isListening) {
                            recognizer?.stopListening()
                        } else if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
                            startListening()
                        } else {
                            micPermission.launch(Manifest.permission.RECORD_AUDIO)
                        }
                    },
                )
            }
            LazyColumn(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 12.dp), state = listState) {
                if (entries.isEmpty()) item {
                    Text(
                        "Tippe auf den Ring und sag zum Beispiel:\n„Öffne Minecraft am PC“\n„Google Wetter Berlin“\n„Spiel Musik von Adele“",
                        modifier = Modifier.padding(16.dp),
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
                items(entries) { e -> Bubble(e) }
            }
            Row(Modifier.fillMaxWidth().padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = { pickFile.launch(arrayOf("*/*")) }) { Icon(Icons.Filled.AttachFile, "Datei senden") }
                OutlinedTextField(
                    value = input, onValueChange = { input = it }, modifier = Modifier.weight(1f),
                    placeholder = { Text("Befehl tippen …") }, singleLine = true,
                    trailingIcon = {
                        IconButton(onClick = { app.ask(input, speak = false); input = "" }, enabled = input.isNotBlank()) {
                            Icon(Icons.AutoMirrored.Filled.Send, "Senden")
                        }
                    },
                )

            }
        }

        share?.let { uri ->
            AlertDialog(
                onDismissRequest = { pendingShare.value = null },
                title = { Text("An welches Gerät?") },
                text = {
                    Column {
                        if (devices.isEmpty()) Text("Gerade ist kein anderes Gerät verbunden.")
                        devices.forEach { d ->
                            Text(
                                d.name,
                                modifier = Modifier.fillMaxWidth()
                                    .clickable {
                                        pendingShare.value = null
                                        scope.launch { sendFile(uri, d.name) }
                                    }
                                    .padding(vertical = 12.dp),
                                style = MaterialTheme.typography.titleMedium,
                            )
                        }
                    }
                },
                confirmButton = { TextButton(onClick = { pendingShare.value = null }) { Text("Abbrechen") } },
            )
        }
    }

    @Composable
    private fun Bubble(e: Entry) {
        val colors = MaterialTheme.colorScheme
        Row(
            Modifier.fillMaxWidth().padding(vertical = 4.dp),
            horizontalArrangement = if (e.fromUser) Arrangement.End else Arrangement.Start,
        ) {
            Text(
                e.text,
                modifier = Modifier
                    .widthIn(max = 320.dp)
                    .background(
                        when {
                            e.fromUser -> colors.primaryContainer
                            !e.ok -> colors.errorContainer
                            else -> colors.surfaceVariant
                        },
                        RoundedCornerShape(16.dp),
                    )
                    .then(if (e.file != null) Modifier.clickable { openFile(e.file) } else Modifier)
                    .padding(horizontal = 14.dp, vertical = 10.dp),
                color = when {
                    e.fromUser -> colors.onPrimaryContainer
                    !e.ok -> colors.onErrorContainer
                    else -> colors.onSurfaceVariant
                },
            )
        }
    }
}

@Composable
private fun JarvisTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val scheme = when {
        Build.VERSION.SDK_INT >= 31 && dark -> dynamicDarkColorScheme(context)
        Build.VERSION.SDK_INT >= 31 -> dynamicLightColorScheme(context)
        dark -> darkColorScheme()
        else -> lightColorScheme()
    }
    MaterialTheme(colorScheme = scheme, content = content)
}
