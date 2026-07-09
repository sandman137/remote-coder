package com.remotecoder.engine

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import uniffi.remotecoder_engine.ConnConfigFfi
import uniffi.remotecoder_engine.EngineEventFfi
import uniffi.remotecoder_engine.EngineListener
import uniffi.remotecoder_engine.GridSnapshotFfi
import uniffi.remotecoder_engine.RemoteCoderEngine
import uniffi.remotecoder_engine.PaneInfoFfi
import uniffi.remotecoder_engine.PairedHostFfi
import uniffi.remotecoder_engine.SessionInfoFfi
import uniffi.remotecoder_engine.pairEnroll
import java.io.File

/**
 * The single owner of the Rust [RemoteCoderEngine]. Engine methods are `block_on`
 * synchronous over the FFI, so every call here hops to [Dispatchers.IO]. The
 * engine's event callback is republished as a [SharedFlow] the UI collects.
 */
class RemoteCoderRepository(private val appContext: Context) {

    private var engine: RemoteCoderEngine? = null

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
            pairEnroll(payloadJson, device, keysDir()).also(::saveHost)
        }

    // --- paired-host persistence -----------------------------------------
    // The Rust side persists the device key + host-key pin under keysDir();
    // the connection parameters live here so a cold start can reconnect
    // without re-pairing (enroll tokens are single-use).

    private fun prefs() = appContext.getSharedPreferences("rcoder", Context.MODE_PRIVATE)

    private fun saveHost(h: PairedHostFfi) {
        prefs().edit()
            .putString("host", h.host)
            .putInt("port", h.port.toInt())
            .putString("user", h.user)
            .putString("keyPath", h.keyPath)
            .putString("hostkeyFp", h.hostkeyFp)
            .apply()
    }

    /** The last successfully paired host, or null if never paired. */
    fun savedHost(): PairedHostFfi? {
        val p = prefs()
        val host = p.getString("host", null) ?: return null
        return PairedHostFfi(
            host = host,
            port = p.getInt("port", 0).toUShort(),
            user = p.getString("user", "") ?: "",
            keyPath = p.getString("keyPath", "") ?: "",
            hostkeyFp = p.getString("hostkeyFp", "") ?: "",
        )
    }

    /** Drop the stored pairing (e.g. after the host revoked this device). */
    fun forgetHost() {
        prefs().edit().remove("host").remove("port").remove("user")
            .remove("keyPath").remove("hostkeyFp").apply()
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
        val e = RemoteCoderEngine.connect(config)
        e.setListener(listener)
        engine = e
        _connected.value = true
    }

    fun disconnect() {
        engine?.close()
        engine = null
        _connected.value = false
    }

    private fun require(): RemoteCoderEngine =
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
        private var instance: RemoteCoderRepository? = null

        fun get(context: Context): RemoteCoderRepository =
            instance ?: synchronized(this) {
                instance ?: RemoteCoderRepository(context.applicationContext).also { instance = it }
            }
    }
}
