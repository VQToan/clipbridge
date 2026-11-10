package dev.clipbridge

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.widget.Toast
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

class ShareReceiverActivity : Activity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (intent?.action == Intent.ACTION_SEND) {
            handleSendIntent(intent)
        } else {
            finish()
        }
    }

    private fun handleSendIntent(intent: Intent) {
        val type = intent.type ?: ""
        val engine = SyncEngine.getInstance(this)

        if (type.startsWith("text/")) {
            val sharedText = intent.getStringExtra(Intent.EXTRA_TEXT)
            if (!sharedText.isNullOrBlank()) {
                CoroutineScope(Dispatchers.Main).launch {
                    engine.publishText(sharedText)
                    Toast.makeText(this@ShareReceiverActivity, "Shared to ClipBridge", Toast.LENGTH_SHORT).show()
                    finish()
                }
                return
            }
        } else if (type.startsWith("image/")) {
            val imageUri = intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)
            if (imageUri != null) {
                CoroutineScope(Dispatchers.Main).launch {
                    try {
                        val bytes = contentResolver.openInputStream(imageUri)?.use { it.readBytes() }
                        if (bytes != null) {
                            engine.publishImage(bytes)
                            Toast.makeText(this@ShareReceiverActivity, "Image shared to ClipBridge", Toast.LENGTH_SHORT).show()
                        }
                    } catch (e: Exception) {
                        Toast.makeText(this@ShareReceiverActivity, "Failed to read image", Toast.LENGTH_SHORT).show()
                    }
                    finish()
                }
                return
            }
        }

        finish()
    }
}
