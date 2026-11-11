package dev.clipbridge

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED,
            Intent.ACTION_MY_PACKAGE_REPLACED,
            "android.intent.action.QUICKBOOT_POWERON" -> {
                Log.i("ClipBridgeBoot", "System boot or package update detected. Starting ClipboardSyncService...")
                ClipboardSyncService.start(context)
            }
        }
    }
}
