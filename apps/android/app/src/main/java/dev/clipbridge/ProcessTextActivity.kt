package dev.clipbridge

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

class ProcessTextActivity : Activity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val selectedText = intent.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT)?.toString()
        if (!selectedText.isNullOrBlank()) {
            val engine = SyncEngine.getInstance(this)
            CoroutineScope(Dispatchers.Main).launch {
                engine.publishText(selectedText)
                Toast.makeText(this@ProcessTextActivity, "Sent to ClipBridge", Toast.LENGTH_SHORT).show()
                finish()
            }
        } else {
            finish()
        }
    }
}
