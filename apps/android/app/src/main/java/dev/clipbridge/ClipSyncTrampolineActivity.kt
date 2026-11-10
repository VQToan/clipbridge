package dev.clipbridge

import android.app.Activity
import android.os.Bundle
import android.widget.Toast

class ClipSyncTrampolineActivity : Activity() {
    private var hasCaptured = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus && !hasCaptured) {
            hasCaptured = true
            captureAndSync()
        }
    }

    private fun captureAndSync() {
        val engine = SyncEngine.getInstance(this)
        val synced = engine.checkAndSyncClipboard()
        if (synced) {
            Toast.makeText(this, "Clipboard synced to Mac! 🚀", Toast.LENGTH_SHORT).show()
        } else {
            Toast.makeText(this, "Clipboard already synced or empty", Toast.LENGTH_SHORT).show()
        }
        finish()
    }
}
