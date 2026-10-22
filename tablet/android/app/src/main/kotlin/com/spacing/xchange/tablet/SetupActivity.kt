// TW-0 skeleton (kiosk-shared-phone §7.11.4 W-D8; §7.11.5; §7.11.6 W10). The body is TW-2's.
//
// What TW-2 builds here, on res/layout/setup.xml:
// - Scan the admin QR with Google's code scanner (play-services-code-scanner, no camera permission) ⇒
//   PairingUrl.fromScan; or type the code ⇒ PairingUrl.fromCode. `null` ⇒ a worded line, and stay on setup.
// - Hand the URL to TabletActivity, whose WebView loads it; the page pairs (T-D10) and the cookie lands in the
//   WebView's persistent store.
// - Then MeClient.get once more ⇒ PairState.fromMe: Paired ⇒ show "Branch <branchCode> · Tablet <name>" once, then
//   the start page; still Unpaired ⇒ back to setup with a worded line (a refused code).
// - Setup is the only other screen, so M2 holds: nothing here opens a browser or another app.
package com.spacing.xchange.tablet

import android.app.Activity
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.view.KeyEvent
import android.view.WindowManager
import android.widget.Button
import android.widget.EditText
import android.widget.TextView
import android.window.OnBackInvokedDispatcher
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning

class SetupActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
            setShowWhenLocked(true)
            setTurnScreenOn(true)
        } else {
            @Suppress("DEPRECATION")
            window.addFlags(
                WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or
                    WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON
            )
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT) {}
        }

        setContentView(R.layout.setup)

        val scanButton = findViewById<Button>(R.id.setup_scan)
        val codeInput = findViewById<EditText>(R.id.setup_code)
        val pairButton = findViewById<Button>(R.id.setup_pair)
        val messageView = findViewById<TextView>(R.id.setup_message)

        intent?.getStringExtra(EXTRA_MESSAGE)?.let { messageView.text = it }

        val scannerOptions = GmsBarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .build()

        scanButton.setOnClickListener {
            GmsBarcodeScanning.getClient(this, scannerOptions)
                .startScan()
                .addOnSuccessListener { barcode ->
                    val pairUrl = PairingUrl.fromScan(barcode.rawValue.orEmpty(), BuildConfig.XC_ORIGIN)
                    if (pairUrl == null) {
                        messageView.setText(R.string.setup_invalid_qr)
                    } else {
                        launchPairing(pairUrl)
                    }
                }
                .addOnFailureListener {
                    messageView.setText(R.string.setup_scan_failed)
                }
        }

        pairButton.setOnClickListener {
            val pairUrl = PairingUrl.fromCode(codeInput.text?.toString().orEmpty(), BuildConfig.XC_ORIGIN)
            if (pairUrl == null) {
                messageView.setText(R.string.setup_invalid_code)
            } else {
                launchPairing(pairUrl)
            }
        }
    }

    override fun onResume() {
        super.onResume()
        try {
            startLockTask()
        } catch (_: IllegalArgumentException) {
            // Activity not yet in foreground
        }
    }

    @Deprecated("Deprecated in Java")
    override fun onBackPressed() {
        // W3: back does nothing
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
        if (keyCode == KeyEvent.KEYCODE_BACK) return true
        return super.onKeyDown(keyCode, event)
    }

    private fun launchPairing(pairUrl: String) {
        startActivity(
            Intent(this, TabletActivity::class.java)
                .putExtra(TabletActivity.EXTRA_PAIR_URL, pairUrl)
        )
        finish()
    }

    companion object {
        const val EXTRA_MESSAGE: String = "com.spacing.xchange.tablet.MESSAGE"
    }
}
