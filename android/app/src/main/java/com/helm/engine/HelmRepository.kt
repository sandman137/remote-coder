package com.helm.engine

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import uniffi.helm_engine.ConnConfigFfi
import uniffi.helm_engine.EngineEventFfi
import uniffi.helm_engine.EngineListener
import uniffi.helm_engine.GridSnapshotFfi
import uniffi.helm_engine.HelmEngine
import uniffi.helm_engine.PaneInfoFfi
import uniffi.helm_engine.PairedHostFfi
import uniffi.helm_engine.SessionInfoFfi
import uniffi.helm_engine.pairEnroll
import java.io.File

/**
 * The single owner of the Rust [HelmEngine]. Engine methods are `block_on`
 * synchronous over the FFI, so every call here hops to [Dispatchers.IO]. The
 * engine's event callback is republished as a [SharedFlow] the UI collects.
 */
class HelmRepository(private val appContext: Context) {

    private var engine: HelmEngine? = null

    private val _events = MutableSharedFlow<EngineEventFfi>(extraBufferCapacity = 256)
    val events: SharedFlow<EngineEventFfi> = _events.asSharedFlow()

    private val _connected = MutableStateFlow(false)
    val connected: StateFlow<Boolean> = _connected.asStateFlow()

    private val listener = object : EngineListener {
        override fun onEvent(event: EngineEventFfi) {
            _events.tryEmit(event)
        }
    }

    /** App-private key directory (not backed up; allowBackup=false). */
    private fun keysDir(): String =
        File(appContext.filesDir, "keys").apply { mkdirs() }.absolutePath

    suspend fun pair(payloadJson: String, device: String): PairedHostFfi =
        withContext(Dispatchers.IO) {
            pairEnroll(payloadJson, device, keysDir())
        }

    suspend fun connectSsh(host: PairedHostFfi) = withContext(Dispatchers.IO) {
        connect(
            ConnConfigFfi.Ssh(
                host = host.host,
                port = host.port,
                user = host.user,
                keyPath = host.keyPath,
                hostkeyFp = host.hostkeyFp.ifEmpty { null },
            ),
        )
    }

    /** Local transport — only meaningful on the same host (dev/emulator). */
    suspend fun connectLocal(socket: String?) = withContext(Dispatchers.IO) {
        connect(ConnConfigFfi.Local(socket))
    }

    private fun connect(config: ConnConfigFfi) {
        engine?.close()
        val e = HelmEngine.connect(config)
        e.setListener(listener)
        engine = e
        _connected.value = true
    }

    fun disconnect() {
        engine?.close()
        engine = null
        _connected.value = false
    }

    private fun require(): HelmEngine =
        engine ?: throw IllegalStateException("not connected")

    suspend fun listSessions(): List<SessionInfoFfi> = withContext(Dispatchers.IO) {
        require().listSessions()
    }

    suspend fun listPanes(session: String): List<PaneInfoFfi> = withContext(Dispatchers.IO) {
        require().listPanes(session)
    }

    suspend fun snapshot(pane: String, scrollback: UInt): GridSnapshotFfi =
        withContext(Dispatchers.IO) { require().snapshot(pane, scrollback) }

    suspend fun attach(pane: String, cols: UShort, rows: UShort) = withContext(Dispatchers.IO) {
        require().attach(pane, cols, rows)
    }

    suspend fun detach(pane: String) = withContext(Dispatchers.IO) { require().detach(pane) }

    suspend fun resize(pane: String, cols: UShort, rows: UShort) = withContext(Dispatchers.IO) {
        require().resize(pane, cols, rows)
    }

    suspend fun sendKeys(pane: String, keys: String) = withContext(Dispatchers.IO) {
        require().sendKeys(pane, keys)
    }

    suspend fun sendText(pane: String, text: String) = withContext(Dispatchers.IO) {
        require().sendText(pane, text)
    }

    suspend fun pressButton(pane: String, label: String) = withContext(Dispatchers.IO) {
        require().pressButton(pane, label)
    }

    companion object {
        @Volatile
        private var instance: HelmRepository? = null

        fun get(context: Context): HelmRepository =
            instance ?: synchronized(this) {
                instance ?: HelmRepository(context.applicationContext).also { instance = it }
            }
    }
}
