package com.helm

import android.content.Intent
import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.fragment.app.FragmentActivity
import com.helm.notify.SessionForegroundService
import com.helm.ui.HelmTheme
import com.helm.ui.HelmViewModel
import com.helm.ui.PaneListScreen
import com.helm.ui.PaneScreen
import com.helm.ui.PairingScreen
import com.helm.ui.Screen

/**
 * Single-activity Compose host. FragmentActivity so BiometricPrompt works.
 * Notification taps deep-link here via helm://pane/<session>/<paneId>.
 */
class MainActivity : FragmentActivity() {

    private val vm: HelmViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Keep the connection alive in the background (persistent notification).
        SessionForegroundService.start(this)

        setContent {
            HelmTheme {
                val state by vm.state.collectAsState()
                when (state.screen) {
                    is Screen.Pairing -> PairingScreen(
                        error = state.error,
                        onScanned = { vm.pairAndConnect(it, deviceName()) },
                        onDevConnect = { vm.connectLocal(null) },
                    )
                    is Screen.PaneList -> PaneListScreen(
                        session = state.session,
                        panes = state.panes,
                        attention = state.attention,
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
        if (data.scheme == "helm" && data.host == "pane") {
            // /<session>/<paneId>
            val segments = data.pathSegments
            if (segments.size >= 2) {
                val paneId = segments[1]
                vm.state.value.panes.firstOrNull { it.id == paneId }?.let(vm::openPane)
            }
        }
    }

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
}
