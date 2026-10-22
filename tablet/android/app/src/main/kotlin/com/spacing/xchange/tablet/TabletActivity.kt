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

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.view.ActionMode
import android.view.KeyEvent
import android.view.WindowManager
import android.webkit.CookieManager
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Toast
import android.window.OnBackInvokedDispatcher
import java.util.concurrent.atomic.AtomicBoolean

class TabletActivity : Activity() {
    private var webView: WebView? = null
    private val pendingPair = AtomicBoolean(false)

    private val startUrl: String
        get() = BuildConfig.XC_ORIGIN + QueueFlowPolicy.START_PATH

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

        CookieManager.getInstance().setAcceptCookie(true)
        mountWebView(null)
        handleIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        try {
            startLockTask()
        } catch (_: IllegalArgumentException) {
            // Activity not yet in foreground
        }
    }

    override fun onPause() {
        CookieManager.getInstance().flush()
        super.onPause()
    }

    override fun onDestroy() {
        webView?.destroy()
        webView = null
        super.onDestroy()
    }

    @Deprecated("Deprecated in Java")
    override fun onBackPressed() {
        // W3: back does nothing, and the WebView does not go back
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
        if (keyCode == KeyEvent.KEYCODE_BACK) return true
        return super.onKeyDown(keyCode, event)
    }

    override fun onActionModeStarted(mode: ActionMode?) {
        super.onActionModeStarted(mode)
        // W8: selection menu offers nothing that opens another app (share, web search, translate)
        mode?.menu?.clear()
        mode?.finish()
    }

    private fun handleIntent(currentIntent: Intent?) {
        val pairUrl = currentIntent?.getStringExtra(EXTRA_PAIR_URL)
        if (pairUrl != null) {
            currentIntent.removeExtra(EXTRA_PAIR_URL)
            pendingPair.set(true)
            webView?.loadUrl(pairUrl)
            return
        }

        val cookie = CookieManager.getInstance().getCookie(BuildConfig.XC_ORIGIN)
        Thread {
            val res = MeClient.get(BuildConfig.XC_ORIGIN, cookie)
            val state = PairState.fromMe(res.status, res.body)
            runOnUiThread {
                if (isFinishing || isDestroyed) return@runOnUiThread
                when (state) {
                    is PairState.Unpaired -> openSetup(null)
                    is PairState.Paired, is PairState.Unknown -> webView?.loadUrl(startUrl)
                }
            }
        }.start()
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun mountWebView(initialUrl: String?) {
        webView?.destroy()
        val view = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.setSupportMultipleWindows(false)
            settings.javaScriptCanOpenWindowsAutomatically = false
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            isLongClickable = false
            setOnLongClickListener { true }
            CookieManager.getInstance().setAcceptThirdPartyCookies(this, false)

            webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                    if (!request.isForMainFrame) return false
                    return decideNavigation(view, request.url.toString())
                }

                @Deprecated("Deprecated in Java")
                override fun shouldOverrideUrlLoading(view: WebView, url: String): Boolean {
                    return decideNavigation(view, url)
                }

                override fun shouldInterceptRequest(
                    view: WebView,
                    request: WebResourceRequest
                ): WebResourceResponse? {
                    if (pendingPair.get() && request.url.path == "/api/branch-tablet/me") {
                        if (pendingPair.compareAndSet(true, false)) {
                            Thread { verifyPairingAfterLoad() }.start()
                        }
                    }
                    return null
                }

                override fun onPageFinished(view: WebView, url: String?) {
                    CookieManager.getInstance().flush()
                    if (pendingPair.get()) {
                        view.postDelayed({
                            if (pendingPair.compareAndSet(true, false)) {
                                Thread { verifyPairingAfterLoad() }.start()
                            }
                        }, 2_500L)
                    }
                }

                override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
                    // W9: replace crashed WebView on the start page, never close the app
                    mountWebView(startUrl)
                    return true
                }
            }
        }
        webView = view
        setContentView(view)
        if (initialUrl != null) {
            view.loadUrl(initialUrl)
        }
    }

    private fun decideNavigation(view: WebView, url: String): Boolean {
        return when (QueueFlowPolicy.decide(url, BuildConfig.XC_ORIGIN)) {
            NavDecision.Allow -> false
            NavDecision.StartPage -> {
                view.loadUrl(startUrl)
                true
            }
            NavDecision.Ignore -> true
        }
    }

    private fun verifyPairingAfterLoad() {
        CookieManager.getInstance().flush()
        val cookie = CookieManager.getInstance().getCookie(BuildConfig.XC_ORIGIN)
        val res = MeClient.get(BuildConfig.XC_ORIGIN, cookie)
        val state = PairState.fromMe(res.status, res.body)
        runOnUiThread {
            if (isFinishing || isDestroyed) return@runOnUiThread
            when (state) {
                is PairState.Paired -> {
                    Toast.makeText(
                        this,
                        getString(R.string.setup_paired, state.branchCode, state.name),
                        Toast.LENGTH_LONG
                    ).show()
                    webView?.loadUrl(startUrl)
                }
                is PairState.Unpaired -> openSetup(getString(R.string.setup_code_refused))
                is PairState.Unknown -> webView?.loadUrl(startUrl)
            }
        }
    }

    private fun openSetup(message: String?) {
        val next = Intent(this, SetupActivity::class.java)
        if (message != null) {
            next.putExtra(SetupActivity.EXTRA_MESSAGE, message)
        }
        startActivity(next)
    }

    companion object {
        const val EXTRA_PAIR_URL: String = "com.spacing.xchange.tablet.PAIR_URL"
    }
}
