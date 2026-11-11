package dev.clipbridge

import android.accessibilityservice.AccessibilityService
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.accessibility.AccessibilityEvent

class ClipBridgeAccessibilityService : AccessibilityService() {

    private val handler = Handler(Looper.getMainLooper())
    private var lastSyncTime = 0L
    private var clipboardManager: ClipboardManager? = null

    private val clipListener = ClipboardManager.OnPrimaryClipChangedListener {
        Log.i(TAG, "Accessibility service received onPrimaryClipChanged callback")
        scheduleClipboardSync(20L)
    }

    override fun onServiceConnected() {
        super.onServiceConnected()
        Log.i(TAG, "ClipBridgeAccessibilityService connected")
        try {
            clipboardManager = getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
            clipboardManager?.addPrimaryClipChangedListener(clipListener)
        } catch (e: Exception) {
            Log.w(TAG, "Failed to register clip listener in accessibility service", e)
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        try {
            clipboardManager?.removePrimaryClipChangedListener(clipListener)
        } catch (_: Exception) {}
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        if (event == null) return

        when (event.eventType) {
            AccessibilityEvent.TYPE_VIEW_CLICKED,
            AccessibilityEvent.TYPE_VIEW_LONG_CLICKED -> {
                val textList = event.text.map { it.toString().lowercase() }
                val desc = event.contentDescription?.toString()?.lowercase() ?: ""
                val viewId = event.source?.viewIdResourceName?.lowercase() ?: ""
                val className = event.className?.toString()?.lowercase() ?: ""

                val isCopyAction = textList.any {
                    it.contains("copy") || it.contains("sao chép") || it.contains("cut") || it.contains("cắt") || it.contains("chép") || it.contains("复制")
                } ||
                desc.contains("copy") || desc.contains("sao chép") || desc.contains("cut") || desc.contains("cắt") || desc.contains("chép") || desc.contains("复制") ||
                viewId.contains("copy") || viewId.contains("cut") ||
                className.contains("copy")

                if (isCopyAction) {
                    Log.i(TAG, "Detected copy action via Accessibility: text=$textList, desc=$desc, viewId=$viewId")
                    scheduleClipboardSync(30L)
                }
            }
            AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED,
            AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED -> {
                // If user just closed selection or window state changed after copying
            }
        }
    }

    private fun scheduleClipboardSync(delayMs: Long) {
        val now = System.currentTimeMillis()
        if (now - lastSyncTime < 250L) return // Debounce rapid multi-events
        lastSyncTime = now

        // 1. Try direct sync first (0ms latency)
        val engine = SyncEngine.getInstance(this)
        val directSynced = engine.checkAndSyncClipboard()
        if (directSynced) {
            Log.i(TAG, "Direct clipboard sync succeeded with 0ms latency!")
            return
        }

        // 2. If direct access wasn't available in this window context, launch trampoline immediately
        handler.postDelayed({
            try {
                val intent = Intent(this, ClipSyncTrampolineActivity::class.java).apply {
                    flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_NO_ANIMATION
                }
                startActivity(intent)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to launch trampoline from accessibility service", e)
            }
        }, delayMs)
    }

    override fun onInterrupt() {
        Log.d(TAG, "Accessibility service interrupted")
    }

    companion object {
        private const val TAG = "ClipAccessibility"
    }
}
