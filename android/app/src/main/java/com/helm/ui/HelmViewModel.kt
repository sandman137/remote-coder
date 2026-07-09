package com.helm.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.helm.engine.HelmRepository
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.helm_engine.ButtonFfi
import uniffi.helm_engine.EngineEventFfi
import uniffi.helm_engine.GridSnapshotFfi
import uniffi.helm_engine.PaneInfoFfi

sealed interface Screen {
    data object Pairing : Screen
    data object PaneList : Screen
    data class Pane(val pane: PaneInfoFfi) : Screen
}

data class UiState(
    val screen: Screen = Screen.Pairing,
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
)

private val defaultButtons = listOf(
    ButtonFfi("Yes", "y"),
    ButtonFfi("No", "n"),
    ButtonFfi("Enter", "<Enter>"),
    ButtonFfi("Ctrl-C", "<C-c>"),
)

class HelmViewModel(app: Application) : AndroidViewModel(app) {
    private val repo = HelmRepository.get(app)

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** Grid viewport reported by the renderer (cols, rows), drives reflow. */
    private var viewport: Pair<UShort, UShort> = 40.toUShort() to 30.toUShort()

    init {
        viewModelScope.launch {
            repo.events.collect { onEvent(it) }
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
                update { copy(panes = panes) }
            } catch (e: Exception) {
                update { copy(error = e.message) }
            }
        }
    }

    fun openPane(pane: PaneInfoFfi) {
        update { copy(screen = Screen.Pane(pane), grid = null, scrollback = 0u) }
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
                update {
                    copy(
                        attention = attention + ev.pane,
                        buttons = if (ev.buttons.isNotEmpty()) ev.buttons else buttons,
                        status = "⚠ ${ev.agent} is waiting",
                    )
                }
            }
            is EngineEventFfi.AttentionCleared ->
                update { copy(attention = attention - ev.pane) }
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
