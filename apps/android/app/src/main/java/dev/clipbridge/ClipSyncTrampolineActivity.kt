package dev.clipbridge

import android.app.Activity
import android.os.Build
import android.os.Bundle
import android.util.Log

class ClipSyncTrampolineActivity : Activity() {
    private var hasCaptured = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        disableTransitions()
        Log.i(TAG, "ClipSyncTrampolineActivity onCreate")
        captureAndSync()
    }

    override fun onResume() {
        super.onResume()
        if (!hasCaptured) {
            captureAndSync()
        }
    }

    private fun disableTransitions() {
        if (Build.VERSION.SDK_INT >= 34) {
            overrideActivityTransition(OVERRIDE_TRANSITION_OPEN, 0, 0)
            overrideActivityTransition(OVERRIDE_TRANSITION_CLOSE, 0, 0)
        } else {
            @Suppress("DEPRECATION")
            overridePendingTransition(0, 0)
        }
    }

    private fun captureAndSync() {
        if (hasCaptured) return
        hasCaptured = true

        val engine = SyncEngine.getInstance(this)
        val synced = engine.checkAndSyncClipboard()
        Log.i(TAG, "captureAndSync completed: synced=$synced")

        finish()
        disableTransitions()
    }

    companion object {
        private const val TAG = "ClipTrampoline"
    }
}
