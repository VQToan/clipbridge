package dev.clipbridge

import android.accessibilityservice.AccessibilityService
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.accessibility.AccessibilityEvent

class ClipBridgeAccessibilityService : AccessibilityService() {

    private val handler = Handler(Looper.getMainLooper())
    private var lastSyncTime = 0L

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        if (event == null) return

        when (event.eventType) {
            AccessibilityEvent.TYPE_VIEW_CLICKED -> {
                val textList = event.text.map { it.toString().lowercase() }
                val desc = event.contentDescription?.toString()?.lowercase() ?: ""
                val viewId = event.source?.viewIdResourceName?.lowercase() ?: ""

                val isCopyAction = textList.any { it.contains("copy") || it.contains("sao chép") || it.contains("cut") || it.contains("cắt") } ||
                        desc.contains("copy") || desc.contains("sao chép") || desc.contains("cut") || desc.contains("cắt") ||
                        viewId.contains("copy") || viewId.contains("cut")

                if (isCopyAction) {
                    scheduleClipboardSync(150L)
                }
            }
            AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED -> {
                // User is actively highlighting text. If they subsequently copy, it will be detected.
            }
        }
    }

    private fun scheduleClipboardSync(delayMs: Long) {
        val now = System.currentTimeMillis()
        if (now - lastSyncTime < 600L) return // Debounce duplicate events
        lastSyncTime = now

        handler.postDelayed({
            try {
                val intent = Intent(this, ClipSyncTrampolineActivity::class.java).apply {
                    flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
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
