package dev.clipbridge

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.SharedPreferences
import android.os.Build
import android.util.Log
import uniffi.clip_ffi.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File

data class ConnectionStatus(
    val state: State = State.DISCONNECTED,
    val message: String = ""
) {
    enum class State { DISCONNECTED, CONNECTING, CONNECTED, PAUSED }
}

class SyncEngine private constructor(private val context: Context) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val prefs: SharedPreferences = context.getSharedPreferences("clipbridge_prefs", Context.MODE_PRIVATE)
    private val clipboard: ClipboardManager = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    private val _status = MutableStateFlow(ConnectionStatus())
    val status: StateFlow<ConnectionStatus> = _status.asStateFlow()

    private val _recentClips = MutableStateFlow<List<FfiHistoryEntry>>(emptyList())
    val recentClips: StateFlow<List<FfiHistoryEntry>> = _recentClips.asStateFlow()

    var groupKey: FfiGroupKey? = null
        private set
    var deviceId: String = ""
        private set
    var relayUrl: String = "ws://10.0.2.2:8080"
        private set

    private var history: FfiHistory? = null
    private val echoGuard = FfiEchoGuard()
    private var relayClient: FfiRelayClient? = null
    private var syncJob: Job? = null
    private var bleManager: BleSyncManager? = null

    init {
        setupHistory()
        setupBleManager()
        loadConfigOrCreate()
    }

    private fun setupBleManager() {
        bleManager = BleSyncManager(context) { envelope ->
            scope.launch {
                val key = groupKey ?: return@launch
                handleIncomingEnvelope(envelope, key, "BLE")
            }
        }
    }

    private fun setupHistory() {
        val dbFile = File(context.filesDir, "history.db")
        history = try {
            FfiHistory.open(dbFile.absolutePath)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to open history DB", e)
            FfiHistory.openInMemory()
        }
        refreshHistory()
    }

    fun refreshHistory() {
        history?.let { h ->
            try {
                _recentClips.value = h.recent(30u)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to load history", e)
            }
        }
    }

    private fun loadConfigOrCreate() {
        val savedKey = prefs.getString("group_key", null)
        relayUrl = prefs.getString("relay_url", "ws://10.0.2.2:8080") ?: "ws://10.0.2.2:8080"
        deviceId = prefs.getString("device_id", null) ?: ffiNewDeviceId().also {
            prefs.edit().putString("device_id", it).apply()
        }

        if (savedKey != null) {
            try {
                val key = FfiGroupKey.fromBase64url(savedKey)
                groupKey = key
                bleManager?.updateGroupKey(key)
                startSync()
            } catch (e: Exception) {
                Log.e(TAG, "Invalid saved group key", e)
            }
        } else {
            // Initialize new group
            val key = FfiGroupKey.generate()
            groupKey = key
            prefs.edit().putString("group_key", key.toBase64url()).apply()
            bleManager?.updateGroupKey(key)
            startSync()
        }
    }

    fun joinGroup(uriString: String): Boolean {
        return try {
            val pair = FfiPairingInfo.fromUri(uriString)
            val key = pair.groupKey()
            groupKey = key
            relayUrl = pair.relayUrl()
            prefs.edit()
                .putString("group_key", key.toBase64url())
                .putString("relay_url", relayUrl)
                .apply()
            startSync()
            true
        } catch (e: Exception) {
            Log.e(TAG, "Failed to join group via URI: $uriString", e)
            false
        }
    }

    fun getPairingUri(): String? {
        val key = groupKey ?: return null
        return FfiPairingInfo(relayUrl, key).toUri()
    }

    fun startSync() {
        syncJob?.cancel()
        syncJob = scope.launch {
            runRelayLoop()
        }
    }

    fun stopSync() {
        syncJob?.cancel()
        syncJob = null
        _status.value = ConnectionStatus(ConnectionStatus.State.DISCONNECTED)
    }

    suspend fun publishText(text: String) = withContext(Dispatchers.IO) {
        val key = groupKey ?: return@withContext
        try {
            val payload = FfiClipPayload.text(text)
            if (echoGuard.shouldPublish(payload.hash())) {
                val envelope = FfiEnvelope.seal(key, deviceId, payload)

                // 1. Send via BLE to nearby devices
                bleManager?.broadcastEnvelope(envelope)

                // 2. Send via Relay WebSocket
                relayClient?.let { client ->
                    try { client.publish(envelope) } catch (e: Exception) { Log.w(TAG, "Relay publish failed", e) }
                }

                val entry = FfiHistoryEntry(
                    id = envelope.id(),
                    origin = "local",
                    kind = payload.kind(),
                    mime = payload.mime(),
                    hash = payload.hash(),
                    size = payload.data().size.toULong(),
                    createdAt = envelope.createdAt(),
                    data = payload.data(),
                    isRemote = false
                )
                history?.insert(entry)
                refreshHistory()
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to publish text", e)
        }
    }

    suspend fun publishImage(pngBytes: ByteArray) = withContext(Dispatchers.IO) {
        val key = groupKey ?: return@withContext
        val client = relayClient ?: return@withContext
        try {
            val normalized = ffiNormalizeToPng(pngBytes)
            val payload = FfiClipPayload.image(normalized)
            if (echoGuard.shouldPublish(payload.hash())) {
                val envelope = FfiEnvelope.seal(key, deviceId, payload)
                client.publish(envelope)

                val entry = FfiHistoryEntry(
                    id = envelope.id(),
                    origin = "local",
                    kind = payload.kind(),
                    mime = payload.mime(),
                    hash = payload.hash(),
                    size = payload.data().size.toULong(),
                    createdAt = envelope.createdAt(),
                    data = payload.data(),
                    isRemote = false
                )
                history?.insert(entry)
                refreshHistory()
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to publish image", e)
        }
    }

    private suspend fun runRelayLoop() {
        var backoffMs = 1000L
        while (currentCoroutineContext().isActive) {
            val key = groupKey
            if (key == null) {
                delay(1000)
                continue
            }

            _status.value = ConnectionStatus(ConnectionStatus.State.CONNECTING)
            val since = try { history?.lastRemoteEnvelopeId() } catch (e: Exception) { null }

            try {
                val client = FfiRelayClient.connect(relayUrl, deviceId, key, since)
                relayClient = client
                _status.value = ConnectionStatus(ConnectionStatus.State.CONNECTED)
                backoffMs = 1000L

                while (currentCoroutineContext().isActive) {
                    val event = client.nextEvent() ?: break
                    handleRelayEvent(event, key)
                }
            } catch (e: Exception) {
                Log.w(TAG, "Relay disconnected: ${e.message}, retrying in ${backoffMs}ms")
                _status.value = ConnectionStatus(ConnectionStatus.State.DISCONNECTED, e.message ?: "")
            }

            relayClient = null
            delay(backoffMs)
            backoffMs = (backoffMs * 2).coerceAtMost(30000L)
        }
    }

    private fun handleRelayEvent(event: FfiRelayEvent, key: FfiGroupKey) {
        when (event) {
            is FfiRelayEvent.Deliver -> {
                handleIncomingEnvelope(event.envelope, key, "Relay")
            }
            is FfiRelayEvent.Ack -> {
                Log.d(TAG, "Server ACK: ${event.id}")
            }
            is FfiRelayEvent.Error -> {
                Log.w(TAG, "Server error: [${event.code}] ${event.message}")
            }
        }
    }

    private fun handleIncomingEnvelope(env: FfiEnvelope, key: FfiGroupKey, source: String) {
        if (echoGuard.seenEnvelope(env.id())) return
        try {
            val payload = env.open(key)
            echoGuard.markApplied(payload.hash())

            // Android allows background services to write to clipboard
            applyToSystemClipboard(payload)

            val entry = FfiHistoryEntry(
                id = env.id(),
                origin = "${env.sender()} ($source)",
                kind = payload.kind(),
                mime = payload.mime(),
                hash = payload.hash(),
                size = payload.data().size.toULong(),
                createdAt = env.createdAt(),
                data = payload.data(),
                isRemote = true
            )
            history?.insert(entry)
            refreshHistory()
        } catch (e: Exception) {
            Log.e(TAG, "Failed to decrypt envelope ${env.id()} via $source", e)
        }
    }

    private fun applyToSystemClipboard(payload: FfiClipPayload) {
        try {
            when (payload.kind()) {
                FfiClipKind.TEXT -> {
                    val text = payload.asText() ?: String(payload.data(), Charsets.UTF_8)
                    val clip = ClipData.newPlainText("ClipBridge", text)
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                        // Mark normal content (not sensitive password)
                    }
                    clipboard.setPrimaryClip(clip)
                }
                FfiClipKind.IMAGE -> {
                    // Save cached image uri and put in clipboard if supported
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to apply to clipboard", e)
        }
    }

    fun copyToClipboard(entry: FfiHistoryEntry) {
        echoGuard.markApplied(entry.hash)
        when (entry.kind) {
            FfiClipKind.TEXT -> {
                val text = String(entry.data, Charsets.UTF_8)
                clipboard.setPrimaryClip(ClipData.newPlainText("ClipBridge", text))
            }
            FfiClipKind.IMAGE -> {
                // Image handling
            }
        }
    }

    companion object {
        private const val TAG = "ClipBridgeSync"

        @Volatile
        private var instance: SyncEngine? = null

        fun getInstance(context: Context): SyncEngine {
            return instance ?: synchronized(this) {
                instance ?: SyncEngine(context.applicationContext).also { instance = it }
            }
        }
    }
}
