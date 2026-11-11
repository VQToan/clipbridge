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
import kotlinx.coroutines.launch
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
    val scope = rememberCoroutineScope()
    val status by engine.status.collectAsState()
    val clips by engine.recentClips.collectAsState()

    var showPairDialog by remember { mutableStateOf(false) }
    var showJoinDialog by remember { mutableStateOf(false) }
    var showResetDialog by remember { mutableStateOf(false) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("ClipBridge", fontWeight = FontWeight.Bold) },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.primaryContainer,
                    titleContentColor = MaterialTheme.colorScheme.onPrimaryContainer
                ),
                actions = {
                    IconButton(onClick = { showResetDialog = true }) {
                        Icon(Icons.Default.Refresh, contentDescription = "Tạo nhóm mới")
                    }
                }
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
                            Toast.makeText(context, "Đã gửi clipboard sang các thiết bị! 🚀", Toast.LENGTH_SHORT).show()
                        } else {
                            Toast.makeText(context, "Clipboard không có nội dung mới", Toast.LENGTH_SHORT).show()
                        }
                    },
                    modifier = Modifier.weight(1.2f)
                ) {
                    Icon(Icons.Default.CloudUpload, contentDescription = null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text("Sync Now", fontSize = 12.sp)
                }

                OutlinedButton(
                    onClick = { showPairDialog = true },
                    modifier = Modifier.weight(1f)
                ) {
                    Icon(Icons.Default.Share, contentDescription = null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text("Ghép đôi", fontSize = 12.sp)
                }

                OutlinedButton(
                    onClick = { showJoinDialog = true },
                    modifier = Modifier.weight(1f)
                ) {
                    Icon(Icons.Default.GroupAdd, contentDescription = null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text("Tham gia", fontSize = 12.sp)
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

    if (showPairDialog) {
        PairDialog(
            engine = engine,
            onDismiss = { showPairDialog = false }
        )
    }

    if (showJoinDialog) {
        JoinDialog(
            onDismiss = { showJoinDialog = false },
            onJoin = { input, onError ->
                scope.launch {
                    val (success, err) = engine.joinWithCodeOrKey(input)
                    if (success) {
                        Toast.makeText(context, "Đã tham gia nhóm thiết bị thành công! 🎉", Toast.LENGTH_SHORT).show()
                        showJoinDialog = false
                    } else {
                        onError(err ?: "Không thể kết nối")
                    }
                }
            }
        )
    }

    if (showResetDialog) {
        AlertDialog(
            onDismissRequest = { showResetDialog = false },
            title = { Text("Tạo nhóm thiết bị mới?") },
            text = {
                Text(
                    "Tính năng này dùng khi bạn muốn tạo một bộ thiết bị riêng biệt (ví dụ chia sẻ app cho người khác). Các thiết bị đang kết nối vào nhóm cũ sẽ không thể đồng bộ nữa cho đến khi được ghép đôi lại.",
                    fontSize = 13.sp
                )
            },
            confirmButton = {
                Button(
                    colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error),
                    onClick = {
                        engine.createNewGroup()
                        Toast.makeText(context, "Đã tạo nhóm thiết bị mới!", Toast.LENGTH_SHORT).show()
                        showResetDialog = false
                    }
                ) {
                    Text("Tạo nhóm mới")
                }
            },
            dismissButton = {
                TextButton(onClick = { showResetDialog = false }) {
                    Text("Hủy")
                }
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
                    text = "Mã thiết bị: ${engine.deviceId.take(8)}...",
                    fontSize = 11.sp,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.outline
                )
                engine.groupKey?.toBase64url()?.let { k ->
                    Text(
                        text = "Mã nhóm: ${k.take(12)}...",
                        fontSize = 10.sp,
                        fontFamily = FontFamily.Monospace,
                        color = MaterialTheme.colorScheme.outline
                    )
                }
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
fun PairDialog(engine: SyncEngine, onDismiss: () -> Unit) {
    val context = LocalContext.current
    var pairingCode by remember { mutableStateOf<String?>(null) }
    var isLoading by remember { mutableStateOf(true) }
    var errorMessage by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(Unit) {
        val code = engine.createPairingCode()
        if (code != null) {
            pairingCode = code
        } else {
            errorMessage = "Không thể lấy mã ghép đôi từ máy chủ"
        }
        isLoading = false
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Ghép đôi thiết bị mới") },
        text = {
            Column(
                modifier = Modifier.fillMaxWidth(),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                Text(
                    text = "Nhập mã 6 số này trên máy tính hoặc thiết bị khác của bạn để kết nối vào chung nhóm:",
                    fontSize = 12.sp,
                    color = MaterialTheme.colorScheme.outline
                )

                if (isLoading) {
                    CircularProgressIndicator(modifier = Modifier.size(36.dp))
                } else if (pairingCode != null) {
                    val formatted = if (pairingCode!!.length == 6) {
                        "${pairingCode!!.take(3)} ${pairingCode!!.takeLast(3)}"
                    } else pairingCode!!

                    Text(
                        text = formatted,
                        fontSize = 32.sp,
                        fontWeight = FontWeight.Bold,
                        fontFamily = FontFamily.Monospace,
                        letterSpacing = 4.sp,
                        color = MaterialTheme.colorScheme.primary
                    )

                    Text(
                        text = "Mã có hiệu lực trong 10 phút",
                        fontSize = 11.sp,
                        color = MaterialTheme.colorScheme.outline
                    )

                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        OutlinedButton(
                            onClick = {
                                val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                                cm.setPrimaryClip(ClipData.newPlainText("Pairing Code", pairingCode))
                                Toast.makeText(context, "Đã sao chép mã 6 số!", Toast.LENGTH_SHORT).show()
                            },
                            modifier = Modifier.weight(1f)
                        ) {
                            Text("Chép mã", fontSize = 11.sp)
                        }

                        OutlinedButton(
                            onClick = {
                                val uri = engine.getPairingUri()
                                if (uri != null) {
                                    val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                                    cm.setPrimaryClip(ClipData.newPlainText("Pairing URI", uri))
                                    Toast.makeText(context, "Đã sao chép link ghép đôi!", Toast.LENGTH_SHORT).show()
                                }
                            },
                            modifier = Modifier.weight(1f)
                        ) {
                            Text("Chép link", fontSize = 11.sp)
                        }
                    }
                } else {
                    Text(
                        text = errorMessage ?: "Lỗi kết nối",
                        color = MaterialTheme.colorScheme.error,
                        fontSize = 12.sp
                    )
                }
            }
        },
        confirmButton = {
            Button(onClick = onDismiss) {
                Text("Đóng")
            }
        }
    )
}

@Composable
fun JoinDialog(onDismiss: () -> Unit, onJoin: (String, (String) -> Unit) -> Unit) {
    var text by remember { mutableStateOf("") }
    var isJoining by remember { mutableStateOf(false) }
    var errorMsg by remember { mutableStateOf<String?>(null) }

    AlertDialog(
        onDismissRequest = { if (!isJoining) onDismiss() },
        title = { Text("Tham gia nhóm thiết bị") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    "Nhập mã kết nối 6 số (ví dụ: 839 201), mã nhóm (Group Key) hoặc link ghép đôi từ thiết bị khác:",
                    fontSize = 12.sp,
                    color = MaterialTheme.colorScheme.outline
                )

                OutlinedTextField(
                    value = text,
                    onValueChange = {
                        text = it
                        errorMsg = null
                    },
                    singleLine = false,
                    maxLines = 3,
                    placeholder = { Text("Ví dụ: 839 201 hoặc IdJwKRJ...", fontSize = 12.sp) },
                    modifier = Modifier.fillMaxWidth()
                )

                if (errorMsg != null) {
                    Text(
                        text = errorMsg!!,
                        color = MaterialTheme.colorScheme.error,
                        fontSize = 12.sp
                    )
                }
            }
        },
        confirmButton = {
            Button(
                onClick = {
                    isJoining = true
                    errorMsg = null
                    onJoin(text.trim()) { err ->
                        isJoining = false
                        errorMsg = err
                    }
                },
                enabled = text.isNotBlank() && !isJoining
            ) {
                if (isJoining) {
                    CircularProgressIndicator(modifier = Modifier.size(16.dp), color = MaterialTheme.colorScheme.onPrimary)
                } else {
                    Text("Kết nối")
                }
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, enabled = !isJoining) {
                Text("Hủy")
            }
        }
    )
}
