package com.remotecoder.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.remotecoder.engine.RemoteCoderRepository
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.remotecoder_engine.ButtonFfi
import uniffi.remotecoder_engine.EngineEventFfi
import uniffi.remotecoder_engine.GridSnapshotFfi
import uniffi.remotecoder_engine.PaneInfoFfi

sealed interface Screen {
    data object Splash : Screen
    data object Pairing : Screen
    data object PaneList : Screen
    data class Pane(val pane: PaneInfoFfi) : Screen
}

data class UiState(
    val screen: Screen = Screen.Splash,
    val connected: Boolean = false,
    val session: String = "agents",
    val panes: List<PaneInfoFfi> = emptyList(),
    val attention: Set<String> = emptySet(),
    val metadata: Map<String, Map<String, String>> = emptyMap(),
    val grid: GridSnapshotFfi? = null,
    val buttons: List<ButtonFfi> = defaultButtons,
    val scrollback: UInt = 0u,
    val status: String = "",
    val error: String? = null,
    /** Host path of a just-uploaded attachment, pending insertion into the input line. */
    val attachment: String? = null,
)

// Claude Code's own key vocabulary: Esc interrupts, Shift-Tab (BTab) cycles
// permission modes, ↑/↓ walk prompt history. Adapter attention events still
// override these with prompt-specific choices (Yes/No/1/2/…).
private val defaultButtons = listOf(
    ButtonFfi("Esc", "<Escape>"),
    ButtonFfi("Mode", "<BTab>"),
    ButtonFfi("↑", "<Up>"),
    ButtonFfi("↓", "<Down>"),
    ButtonFfi("Enter", "<Enter>"),
    ButtonFfi("Ctrl-C", "<C-c>"),
)

class RemoteCoderViewModel(app: Application) : AndroidViewModel(app) {
    private val repo = RemoteCoderRepository.get(app)

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** Grid viewport reported by the renderer (cols, rows), drives reflow. */
    private var viewport: Pair<UShort, UShort> = 40.toUShort() to 30.toUShort()

    init {
        viewModelScope.launch {
            repo.events.collect { onEvent(it) }
        }
        // Cold start: if a host was paired before, reconnect with the stored
        // key + pinned host key (no re-pairing; enroll tokens are single-use).
        // Otherwise hold the splash briefly, then reveal pairing — unless a
        // deep link has already advanced the screen.
        viewModelScope.launch {
            val saved = repo.savedHost()
            kotlinx.coroutines.delay(1700)
            if (_state.value.screen !is Screen.Splash) return@launch
            if (saved == null) {
                update { copy(screen = Screen.Pairing) }
                return@launch
            }
            try {
                update { copy(status = "reconnecting to ${saved.host}…") }
                repo.connectSsh(saved)
                update { copy(connected = true, screen = Screen.PaneList, status = "connected") }
                refreshPanes()
            } catch (e: Exception) {
                update {
                    copy(screen = Screen.Pairing, error = "reconnect failed: ${e.message} — re-pair below")
                }
            }
        }
    }

    private fun update(block: UiState.() -> UiState) {
        _state.value = _state.value.block()
    }

    fun pairAndConnect(payloadJson: String, device: String) {
        viewModelScope.launch {
            try {
                update { copy(status = "pairing…", error = null) }
                val host = repo.pair(payloadJson, device)
                repo.connectSsh(host)
                update { copy(connected = true, screen = Screen.PaneList, status = "connected") }
                refreshPanes()
            } catch (e: Exception) {
                update { copy(error = e.message ?: "pairing failed") }
            }
        }
    }

    /** Dev/emulator path: connect to a host tmux over adb-reversed loopback. */
    fun connectLocal(socket: String?) {
        viewModelScope.launch {
            try {
                repo.connectLocal(socket)
                update { copy(connected = true, screen = Screen.PaneList) }
                refreshPanes()
            } catch (e: Exception) {
                update { copy(error = e.message) }
            }
        }
    }

    fun refreshPanes() {
        viewModelScope.launch {
            try {
                val panes = repo.listPanes(_state.value.session)
                update { copy(panes = panes, error = null) }
            } catch (e: Exception) {
                update { copy(error = e.message) }
            }
        }
    }

    fun openPane(pane: PaneInfoFfi) {
        // Reset to generic controls; this pane's adapter Attention event
        // refines them (and won't be overridden by other panes now).
        update { copy(screen = Screen.Pane(pane), grid = null, scrollback = 0u, buttons = defaultButtons, status = "") }
        viewModelScope.launch {
            try {
                repo.attach(pane.id, viewport.first, viewport.second)
            } catch (e: Exception) {
                update { copy(error = e.message) }
            }
        }
    }

    fun closePane() {
        (_state.value.screen as? Screen.Pane)?.let { s ->
            viewModelScope.launch { runCatching { repo.detach(s.pane.id) } }
        }
        update { copy(screen = Screen.PaneList, grid = null) }
        refreshPanes()
    }

    fun setViewport(cols: UShort, rows: UShort) {
        if (viewport.first == cols && viewport.second == rows) return
        viewport = cols to rows
        (_state.value.screen as? Screen.Pane)?.let { s ->
            viewModelScope.launch { runCatching { repo.resize(s.pane.id, cols, rows) } }
        }
    }

    fun pressButton(label: String) = withPane { pane ->
        val keys = _state.value.buttons.firstOrNull { it.label == label }?.keys
        if (keys != null) repo.sendKeys(pane.id, keys) else repo.pressButton(pane.id, label)
    }

    fun sendText(text: String) = withPane { pane -> repo.sendText(pane.id, text) }

    /**
     * Upload a picked file to the host over the broker channel; the stored
     * path lands in [UiState.attachment] for the input line to absorb —
     * Claude Code reads file/image paths straight from the prompt.
     */
    fun attachFile(uri: android.net.Uri) {
        val resolver = getApplication<Application>().contentResolver
        viewModelScope.launch {
            try {
                update { copy(status = "uploading attachment…", error = null) }
                val (name, bytes) = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
                    var display = "attachment"
                    resolver.query(uri, null, null, null, null)?.use { c ->
                        val i = c.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                        if (i >= 0 && c.moveToFirst()) display = c.getString(i) ?: display
                    }
                    val data = resolver.openInputStream(uri)?.use { it.readBytes() }
                        ?: throw IllegalStateException("cannot read $uri")
                    display to data
                }
                require(bytes.size <= 16 * 1024 * 1024) { "attachment exceeds 16 MB" }
                val path = repo.uploadAttachment(name, bytes)
                update { copy(status = "", attachment = path) }
            } catch (e: Exception) {
                update { copy(status = "", error = "attach failed: ${e.message}") }
            }
        }
    }

    /** The input line has absorbed [UiState.attachment]. */
    fun consumeAttachment() = update { copy(attachment = null) }

    fun scrollBy(delta: Int) {
        val next = (_state.value.scrollback.toLong() + delta).coerceIn(0, 5000).toUInt()
        update { copy(scrollback = next) }
        pollScrollback()
    }

    private fun pollScrollback() = withPane { pane ->
        if (_state.value.scrollback > 0u) {
            val grid = repo.snapshot(pane.id, _state.value.scrollback)
            update { copy(grid = grid) }
        }
    }

    private fun withPane(block: suspend (PaneInfoFfi) -> Unit) {
        val pane = (_state.value.screen as? Screen.Pane)?.pane ?: return
        viewModelScope.launch {
            try {
                block(pane)
            } catch (e: Exception) {
                update { copy(error = e.message) }
            }
        }
    }

    private fun onEvent(ev: EngineEventFfi) {
        when (ev) {
            is EngineEventFfi.Grid -> {
                val current = _state.value.screen
                if (current is Screen.Pane && current.pane.id == ev.pane &&
                    _state.value.scrollback == 0u
                ) {
                    update { copy(grid = ev.snapshot) }
                }
            }
            is EngineEventFfi.Panes ->
                if (ev.session == _state.value.session) update { copy(panes = ev.panes) }
            is EngineEventFfi.Attention -> {
                // Track every waiting pane for the list badges, but only swap
                // the open pane's buttons/status when the attention is for it —
                // a background agent's prompt must not hijack the current view.
                val isCurrent = (_state.value.screen as? Screen.Pane)?.pane?.id == ev.pane
                update {
                    copy(
                        attention = attention + ev.pane,
                        buttons = if (isCurrent && ev.buttons.isNotEmpty()) ev.buttons else buttons,
                        status = if (isCurrent) "⚠ ${ev.agent} is waiting" else status,
                    )
                }
            }
            is EngineEventFfi.AttentionCleared -> {
                val isCurrent = (_state.value.screen as? Screen.Pane)?.pane?.id == ev.pane
                update {
                    copy(
                        attention = attention - ev.pane,
                        status = if (isCurrent && status.startsWith("⚠")) "" else status,
                    )
                }
            }
            is EngineEventFfi.Metadata -> {
                val fields = ev.fields.associate { it.field to it.value }
                update {
                    copy(metadata = metadata + (ev.pane to ((metadata[ev.pane] ?: emptyMap()) + fields)))
                }
            }
            is EngineEventFfi.Reconnecting -> update { copy(status = "reconnecting…") }
            is EngineEventFfi.Connected -> update { copy(status = "") }
            is EngineEventFfi.Error -> update { copy(error = ev.message) }
            else -> {}
        }
    }

    fun clearError() = update { copy(error = null) }
}
