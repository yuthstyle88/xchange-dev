// TW-0 skeleton (kiosk-shared-phone §7.11.4 W-D2, W-D5, W-D8; §7.11.5; §7.11.6 W1-W3, W5-W9, W11). The body is
// TW-2's. It is glue and holds no rule: the rules are QueueFlowPolicy, PairState and PairingUrl in :core.
//
// What TW-2 builds here:
// - At launch: MeClient.get(BuildConfig.XC_ORIGIN, the WebView's cookie) once ⇒ PairState.fromMe.
//   Unpaired ⇒ SetupActivity (W10). Paired or Unknown ⇒ the start page, BuildConfig.XC_ORIGIN +
//   QueueFlowPolicy.START_PATH (W1, W11).
// - One full-screen WebView: JavaScript and DOM storage on; setSupportMultipleWindows(false), so `target="_blank"`
//   runs through the same rule; no download listener, no file chooser, no share handler (W-D5).
// - shouldOverrideUrlLoading for the main frame ⇒ QueueFlowPolicy.decide: Allow loads, StartPage loads the start
//   page, Ignore does nothing (W5-W7). Never hand a URL to another app.
// - onResume: startLockTask() (W-D2, W1); back does nothing, and the WebView does not go back (W3).
// - FLAG_KEEP_SCREEN_ON, and show over the lock screen (W-D2, W1).
// - onRenderProcessGone: a new WebView on the start page, never a closed app (W9).
// - The text selection menu offers nothing that opens another app (W8).
package com.spacing.xchange.tablet

import android.app.Activity
import android.os.Bundle

class TabletActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        TODO("TW-2")
    }
}
