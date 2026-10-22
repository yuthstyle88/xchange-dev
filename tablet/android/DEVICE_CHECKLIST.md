# Branch Tablet Wrapper — Device Checklist (`kiosk-shared-phone.md` §7.11.6)

Run this checklist on the real Android branch tablet before each release, and post the results on the PR.
Rows W5–W7 and W10's URL/state parsing are also covered by JVM unit tests (`./gradlew test`). W12's label is web work (§7.11 W-D9) with no web test on `main` yet, so check it here.

## Device Prerequisites

1. Set a screen-lock PIN on the tablet (*Settings → Security → Screen lock*).
2. Turn on **App pinning** and enable **Ask for PIN before unpinning** (*Settings → Security → Advanced / More security settings → App pinning*).
3. Install the signed `com.spacing.xchange.tablet` APK.

## Checklist (W1–W12 + Reboot & Notification Shade)

| Row | Given | When | Expected | Pass / Fail |
|---|---|---|---|---|
| **W1** | A paired wrapper | Launch **XCHANGE Tablet** | `<origin>/app/queue` fills the screen with no address bar; Android prompts staff to pin the app on launch; the screen stays on (`FLAG_KEEP_SCREEN_ON`) and shows over the lock screen if woken from sleep. | [ ] |
| **W2** | Wrapper is pinned (`startLockTask()`) | Press Home, Recents, or App Switch | Nothing happens; the queue page remains on screen. | [ ] |
| **W3** | On any screen in the queue flow (or on Setup) | Press the Back button or swipe the system Back gesture | Nothing happens: the app does not close and the WebView does not navigate back. | [ ] |
| **W4** | Wrapper is pinned with *Ask for PIN before unpinning* enabled | Perform the unpin gesture (hold Back + Recents, or swipe up and hold on gesture nav) | Device locks / prompts for the tablet's screen-lock PIN; entering a wrong PIN or cancelling keeps the customer out of the OS. | [ ] |
| **W5** | On the queue page | Navigate within `https://<origin>` to `/app/queue`, `/app/sign-in`, or `/app/sign-up` (with any query or fragment) | Navigation loads normally inside the WebView. *(Also verified in CI by `PolicyCasesTest`.)* | [ ] |
| **W6** | On the queue page | Tap any shell link to another path on `<origin>` (e.g., `/app/history`, `/app/profile`, `/app/bookings`, `/app`) or a `target="_blank"` link | WebView loads the start page (`<origin>/app/queue`) instead. *(Also verified in CI by `PolicyCasesTest`.)* | [ ] |
| **W7** | On the queue page | Trigger a navigation to another host, `http://<origin>`, or `intent:`, `tel:`, `mailto:`, `blob:`, `data:`, `file:` (e.g., the QR download button) | Navigation is ignored; no external browser, dialer, share sheet, or download opens. *(Also verified in CI by `PolicyCasesTest`.)* | [ ] |
| **W8** | On the queue page | Long-press any text on the page | No selection menu item appears that can open another app (no Share, Web Search, or Translate). | [ ] |
| **W9** | On the queue page | Crash the WebView renderer (e.g., `adb shell am crash com.google.android.webview` or kill the `:sandboxed_process` PID) | `onRenderProcessGone` replaces the WebView and reloads `<origin>/app/queue`; the app does not close or unpin. | [ ] |
| **W10** | A freshly installed wrapper, or a tablet whose pairing was revoked in Admin (`GET /api/branch-tablet/me` → `paired: false`) | Launch the app, then test: (a) scanning a QR for another origin or without `#pair=`, (b) entering an expired/invalid code, (c) scanning a valid admin pairing QR or typing a valid 10-character code | Opens `SetupActivity`. (a) Refused on Setup with an error message. (b) Loads in WebView, `me` remains `paired: false`, returns to Setup with a refusal message. (c) Pairs via `#pair=<CODE>`, shows `"Branch <code> · Tablet <name>"` once, and opens `<origin>/app/queue`. Subsequent launches go straight to `<origin>/app/queue`. | [ ] |
| **W11** | A paired tablet with Wi-Fi / network turned off (`me` unreachable or non-2xx) | Launch the app | Opens `<origin>/app/queue` as is; does **not** fall back to `SetupActivity` (Setup only appears on explicit `paired: false`). | [ ] |
| **W12** | A paired tablet on `<origin>/app/queue` | Page finishes loading | Displays the `"<code> · <name>"` tablet label (e.g., `"MAIN · Front-1"`). | [ ] |
| **Reboot / Restart** | A paired tablet | Force-stop the app or reboot the tablet, then launch **XCHANGE Tablet** again | Persistent `CookieManager` retains `__Host-xc_tablet`; `me` returns `paired: true` and the app opens `<origin>/app/queue` without asking to pair again. | [ ] |
| **Notification Shade** | Wrapper is pinned | Swipe down from the top edge of the screen | Notification shade and Quick Settings tiles cannot be pulled down to open Settings or leave the app while pinned. | [ ] |
