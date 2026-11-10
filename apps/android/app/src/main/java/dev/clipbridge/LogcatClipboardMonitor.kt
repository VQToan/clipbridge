package dev.clipbridge

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.util.Log
import kotlinx.coroutines.*
import java.io.BufferedReader
import java.io.InputStreamReader

class LogcatClipboardMonitor(private val context: Context) {

    private var job: Job? = null
    private var process: Process? = null
    private var lastTriggerTime = 0L

    fun start(scope: CoroutineScope) {
        val hasPermission = context.checkSelfPermission(Manifest.permission.READ_LOGS) == PackageManager.PERMISSION_GRANTED
        if (!hasPermission) {
            Log.i(TAG, "READ_LOGS permission not granted, logcat monitor inactive")
            return
        }

        job = scope.launch(Dispatchers.IO) {
            try {
                Log.i(TAG, "Starting logcat background clipboard monitor...")
                // Clear old buffer and monitor system/main logs
                Runtime.getRuntime().exec(arrayOf("logcat", "-c")).waitFor()
                val proc = Runtime.getRuntime().exec(arrayOf("logcat", "-v", "time", "-b", "system", "-b", "main"))
                process = proc

                val reader = BufferedReader(InputStreamReader(proc.inputStream))
                while (isActive) {
                    val l = reader.readLine() ?: break
                    if (isClipboardLogLine(l)) {
                        triggerSync()
                    }
                }
            } catch (e: Exception) {
                if (isActive) {
                    Log.w(TAG, "Logcat monitor encountered error", e)
                }
            }
        }
    }

    private fun isClipboardLogLine(line: String): Boolean {
        val lower = line.lowercase()
        return (lower.contains("clipboardservice") && (lower.contains("setprimaryclip") || lower.contains("notifyprimaryclipchanged") || lower.contains("primaryclipchanged"))) ||
                lower.contains("semclipboard") ||
                (lower.contains("clipboard") && lower.contains("primaryclip"))
    }

    private fun triggerSync() {
        val now = System.currentTimeMillis()
        if (now - lastTriggerTime < 600L) return
        lastTriggerTime = now

        try {
            val intent = Intent(context, ClipSyncTrampolineActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
            }
            context.startActivity(intent)
        } catch (e: Exception) {
            Log.w(TAG, "Failed to launch trampoline from logcat monitor", e)
        }
    }

    fun stop() {
        job?.cancel()
        process?.destroy()
        process = null
    }

    companion object {
        private const val TAG = "LogcatClipMonitor"
    }
}
