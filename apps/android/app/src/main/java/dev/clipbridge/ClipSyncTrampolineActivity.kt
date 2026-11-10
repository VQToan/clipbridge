package dev.clipbridge

import android.app.Activity
import android.content.ClipboardManager
import android.content.Context
import android.os.Bundle
import android.widget.Toast
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

class ClipSyncTrampolineActivity : Activity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            captureAndSync()
        }
    }

    private fun captureAndSync() {
        val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val clip = clipboard.primaryClip

        if (clip != null && clip.itemCount > 0) {
            val text = clip.getItemAt(0).coerceToText(this)?.toString()
            if (!text.isNullOrBlank()) {
                val engine = SyncEngine.getInstance(this)
                CoroutineScope(Dispatchers.Main).launch {
                    engine.publishText(text)
                    Toast.makeText(this@ClipSyncTrampolineActivity, "Clip synced to ClipBridge", Toast.LENGTH_SHORT).show()
                    finish()
                }
                return
            }
        }

        Toast.makeText(this, "Clipboard is empty", Toast.LENGTH_SHORT).show()
        finish()
    }
}
