package com.remotecoder

import android.content.Intent
import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.fragment.app.FragmentActivity
import com.remotecoder.notify.SessionForegroundService
import com.remotecoder.ui.RemoteCoderTheme
import com.remotecoder.ui.RemoteCoderViewModel
import com.remotecoder.ui.PaneListScreen
import com.remotecoder.ui.PaneScreen
import com.remotecoder.ui.PairingScreen
import com.remotecoder.ui.Screen
import com.remotecoder.ui.SplashScreen

/**
 * Single-activity Compose host. FragmentActivity so BiometricPrompt works.
 * Notification taps deep-link here via remotecoder://pane/<session>/<paneId>.
 */
class MainActivity : FragmentActivity() {

    private val vm: RemoteCoderViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Keep the connection alive in the background (persistent notification).
        SessionForegroundService.start(this)

        setContent {
            RemoteCoderTheme {
                val state by vm.state.collectAsState()
                when (state.screen) {
                    is Screen.Splash -> SplashScreen(
                        status = state.status.ifEmpty { "connecting" },
                    )
                    is Screen.Pairing -> PairingScreen(
                        error = state.error,
                        onScanned = { vm.pairAndConnect(it, deviceName()) },
                        onDevConnect = { vm.connectLocal(null) },
                    )
                    is Screen.PaneList -> PaneListScreen(
                        session = state.session,
                        panes = state.panes,
                        attention = state.attention,
                        error = state.error,
                        onOpen = vm::openPane,
                        onRefresh = vm::refreshPanes,
                    )
                    is Screen.Pane -> PaneScreen(
                        state = state,
                        onBack = vm::closePane,
                        onButton = vm::pressButton,
                        onSend = vm::sendText,
                        onScroll = vm::scrollBy,
                        onStt = { startSpeechInput() },
                        onAttach = { attachLauncher.launch("*/*") },
                        onAttachmentConsumed = vm::consumeAttachment,
                        onViewport = vm::setViewport,
                    )
                }
            }
        }
        handleDeepLink(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleDeepLink(intent)
    }

    private fun handleDeepLink(intent: Intent?) {
        val data = intent?.data ?: return
        when (data.host) {
            "pane" -> {
                // /<session>/<paneId>
                val segments = data.pathSegments
                if (segments.size >= 2) {
                    val paneId = segments[1]
                    vm.state.value.panes.firstOrNull { it.id == paneId }?.let(vm::openPane)
                }
            }
            // Debug-only: drive the REAL pairing+connect path with a payload
            // (the QR scanner can't be automated over adb). Not available in
            // release builds. `adb shell am start -a android.intent.action.VIEW
            //   -d remotecoder://connect --es payload '<json>' --es device emu`
            "connect" -> if (isDebuggable()) {
                val payload = intent.getStringExtra("payload")
                    ?: data.getQueryParameter("payload")
                val device = intent.getStringExtra("device") ?: "emu"
                if (!payload.isNullOrEmpty()) vm.pairAndConnect(payload, device)
            }
        }
    }

    private fun isDebuggable(): Boolean =
        applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0

    private fun deviceName(): String =
        (android.os.Build.MODEL ?: "phone").replace(Regex("[^A-Za-z0-9]"), "-").lowercase()

    /** Android STT via the system recognizer; result feeds the input line. */
    private fun startSpeechInput() {
        val intent = Intent(android.speech.RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
            putExtra(
                android.speech.RecognizerIntent.EXTRA_LANGUAGE_MODEL,
                android.speech.RecognizerIntent.LANGUAGE_MODEL_FREE_FORM,
            )
        }
        runCatching { sttLauncher.launch(intent) }
    }

    private val sttLauncher = registerForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        val text = result.data
            ?.getStringArrayListExtra(android.speech.RecognizerIntent.EXTRA_RESULTS)
            ?.firstOrNull()
        if (!text.isNullOrEmpty()) vm.sendText(text)
    }

    /** Attachment picker (any file type); the upload path lands in the prompt. */
    private val attachLauncher = registerForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.GetContent(),
    ) { uri -> if (uri != null) vm.attachFile(uri) }
}
