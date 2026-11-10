package dev.clipbridge

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import uniffi.clip_ffi.FfiClipKind
import uniffi.clip_ffi.FfiHistoryEntry
import java.text.SimpleDateFormat
import java.util.*

class MainActivity : ComponentActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val engine = SyncEngine.getInstance(this)

        // Handle incoming pairing link via deep link
        intent?.data?.let { uri ->
            if (uri.scheme == "clipbridge" && uri.host == "pair") {
                val joined = engine.joinGroup(uri.toString())
                if (joined) {
                    Toast.makeText(this, "Joined group successfully!", Toast.LENGTH_SHORT).show()
                }
            }
        }

        // Start background service
        ClipboardSyncService.start(this)

        setContent {
            MaterialTheme {
                MainScreen(engine = engine)
            }
        }
    }

    override fun onResume() {
        super.onResume()
        val engine = SyncEngine.getInstance(this)
        engine.registerClipListener()
        engine.checkAndSyncClipboard()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            SyncEngine.getInstance(this).checkAndSyncClipboard()
        }
    }

    override fun onPause() {
        super.onPause()
        SyncEngine.getInstance(this).unregisterClipListener()
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainScreen(engine: SyncEngine) {
    val context = LocalContext.current
    val status by engine.status.collectAsState()
    val clips by engine.recentClips.collectAsState()
    var showJoinDialog by remember { mutableStateOf(false) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("ClipBridge", fontWeight = FontWeight.Bold) },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.primaryContainer,
                    titleContentColor = MaterialTheme.colorScheme.onPrimaryContainer
                )
            )
        }
    ) { paddingValues ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(paddingValues)
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            // Status Card
            StatusCard(status = status, engine = engine)

            // Actions Row
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                Button(
                    onClick = {
                        val synced = engine.checkAndSyncClipboard()
                        if (synced) {
                            Toast.makeText(context, "Clipboard synced to peers! 🚀", Toast.LENGTH_SHORT).show()
                        } else {
                            Toast.makeText(context, "Clipboard already synced or empty", Toast.LENGTH_SHORT).show()
                        }
                    },
                    modifier = Modifier.weight(1.2f)
                ) {
                    Icon(Icons.Default.CloudUpload, contentDescription = null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text("Sync Now", fontSize = 12.sp)
                }

                OutlinedButton(
                    onClick = {
                        val uri = engine.getPairingUri()
                        if (uri != null) {
                            val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                            cm.setPrimaryClip(ClipData.newPlainText("ClipBridge Pairing", uri))
                            Toast.makeText(context, "Pairing link copied!", Toast.LENGTH_SHORT).show()
                        }
                    },
                    modifier = Modifier.weight(1f)
                ) {
                    Icon(Icons.Default.Share, contentDescription = null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text("Share", fontSize = 12.sp)
                }

                OutlinedButton(
                    onClick = { showJoinDialog = true },
                    modifier = Modifier.weight(1f)
                ) {
                    Icon(Icons.Default.GroupAdd, contentDescription = null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text("Join...", fontSize = 12.sp)
                }
            }

            // Recent Clips Header
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text("Recent Clips (${clips.size})", style = MaterialTheme.typography.titleMedium)
                IconButton(onClick = { engine.refreshHistory() }) {
                    Icon(Icons.Default.Refresh, contentDescription = "Refresh")
                }
            }

            // Clips List
            if (clips.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .weight(1f),
                    contentAlignment = Alignment.Center
                ) {
                    Text(
                        "No clips yet.\nCopy something or use the Quick Settings Tile to sync.",
                        color = MaterialTheme.colorScheme.outline
                    )
                }
            } else {
                LazyColumn(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    items(clips, key = { it.id }) { entry ->
                        ClipItemCard(entry = entry, onClick = { engine.copyToClipboard(entry) })
                    }
                }
            }
        }
    }

    if (showJoinDialog) {
        JoinDialog(
            onDismiss = { showJoinDialog = false },
            onJoin = { uri ->
                val ok = engine.joinGroup(uri)
                if (ok) showJoinDialog = false
            }
        )
    }
}

@Composable
fun StatusCard(status: ConnectionStatus, engine: SyncEngine) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = when (status.state) {
                ConnectionStatus.State.CONNECTED -> Color(0xFFE8F5E9)
                ConnectionStatus.State.CONNECTING -> Color(0xFFFFF3E0)
                ConnectionStatus.State.DISCONNECTED -> Color(0xFFFFEBEE)
                ConnectionStatus.State.PAUSED -> Color(0xFFF5F5F5)
            }
        )
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Icon(
                imageVector = when (status.state) {
                    ConnectionStatus.State.CONNECTED -> Icons.Default.CheckCircle
                    ConnectionStatus.State.CONNECTING -> Icons.Default.Sync
                    ConnectionStatus.State.DISCONNECTED -> Icons.Default.Error
                    ConnectionStatus.State.PAUSED -> Icons.Default.Pause
                },
                contentDescription = null,
                tint = when (status.state) {
                    ConnectionStatus.State.CONNECTED -> Color(0xFF2E7D32)
                    ConnectionStatus.State.CONNECTING -> Color(0xFFEF6C00)
                    ConnectionStatus.State.DISCONNECTED -> Color(0xFFC62828)
                    ConnectionStatus.State.PAUSED -> Color.Gray
                }
            )

            Column {
                Text(
                    text = status.state.name,
                    fontWeight = FontWeight.Bold,
                    fontSize = 14.sp
                )
                Text(
                    text = "Device ID: ${engine.deviceId.take(8)}...",
                    fontSize = 11.sp,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.outline
                )
            }
        }
    }
}

@Composable
fun ClipItemCard(entry: FfiHistoryEntry, onClick: () -> Unit) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable { onClick() }
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.Top
        ) {
            Icon(
                imageVector = if (entry.kind == FfiClipKind.TEXT) Icons.Default.TextFields else Icons.Default.Image,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary
            )

            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (entry.kind == FfiClipKind.TEXT) {
                    val text = String(entry.data, Charsets.UTF_8)
                    Text(
                        text = text,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        fontSize = 13.sp
                    )
                } else {
                    Text(
                        text = "Image (${entry.size} bytes)",
                        fontWeight = FontWeight.Medium,
                        fontSize = 13.sp
                    )
                }

                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        text = if (entry.isRemote) "From remote" else "Local",
                        fontSize = 10.sp,
                        color = MaterialTheme.colorScheme.outline
                    )
                    Text("•", fontSize = 10.sp, color = MaterialTheme.colorScheme.outline)
                    Text(
                        text = SimpleDateFormat("HH:mm:ss", Locale.getDefault()).format(Date(entry.createdAt)),
                        fontSize = 10.sp,
                        color = MaterialTheme.colorScheme.outline
                    )
                }
            }
        }
    }
}

@Composable
fun JoinDialog(onDismiss: () -> Unit, onJoin: (String) -> Unit) {
    var text by remember { mutableStateOf("") }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Join Group") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Paste pairing URI (clipbridge://pair?d=...):", fontSize = 12.sp)
                OutlinedTextField(
                    value = text,
                    onValueChange = { text = it },
                    singleLine = false,
                    maxLines = 4,
                    modifier = Modifier.fillMaxWidth()
                )
            }
        },
        confirmButton = {
            Button(
                onClick = { onJoin(text.trim()) },
                enabled = text.isNotBlank()
            ) {
                Text("Join")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        }
    )
}
