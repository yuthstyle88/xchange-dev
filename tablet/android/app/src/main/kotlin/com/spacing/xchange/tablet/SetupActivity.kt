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
import android.os.Bundle

class SetupActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        TODO("TW-2")
    }
}
