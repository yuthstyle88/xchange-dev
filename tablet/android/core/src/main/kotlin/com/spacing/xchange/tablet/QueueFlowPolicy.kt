// TW-0 skeleton (kiosk-shared-phone §7.11.4 W-D5, W-D6; §7.11.5; §7.11.6 W5-W7). The body is TW-1's.
package com.spacing.xchange.tablet

/** What the wrapper does with one top-level navigation (W-D5). */
enum class NavDecision {
    /** Load the URL as asked. */
    Allow,

    /** Load the start page, `<origin>/app/queue`, instead. */
    StartPage,

    /** Do nothing, and never hand the URL to another app. */
    Ignore,
}

/**
 * M2's one rule (W-D5), for every top-level navigation: the WebView's `shouldOverrideUrlLoading` on Android, and
 * `decidePolicyFor` on iPad. Subresources (`/ui/...`, `/api/...` fetches) are not navigations and never reach it.
 *
 * `decide(url, origin)`, where `origin` is the build's own `https://host` or `https://host:port`, with no path and no
 * trailing slash (`BuildConfig.XC_ORIGIN`):
 * - scheme `https`, the same host and port as `origin`, and a path exactly one of [FLOW_PATHS], with any query or
 *   fragment ⇒ [NavDecision.Allow] (W5). Exact: `/app/queue/`, `/app/queueX` and `/app/queue/../history` are not
 *   flow paths, and paths are case-sensitive.
 * - scheme `https` and the same host and port as `origin`, any other path (or none) ⇒ [NavDecision.StartPage] (W6).
 * - anything else ⇒ [NavDecision.Ignore] (W7): another host (also our host inside user info, or as a prefix or a
 *   suffix of another host), another port, `http`, `intent:`, `tel:`, `mailto:`, `blob:`, `data:`, `file:`,
 *   `javascript:`, a URL with no host, and text that does not parse. It never throws.
 *
 * The cases are `tablet/policy-cases.json` (W-D7), shared with the iPad app.
 */
object QueueFlowPolicy {
    /** W-D6: the queue flow as built (`QueueRequest.tsx`, `SignIn.tsx`). */
    val FLOW_PATHS: Set<String> = setOf("/app/queue", "/app/sign-in", "/app/sign-up")

    /** W-D6: the start page is `<origin>` + this; the page picks the tablet's branch itself. */
    const val START_PATH: String = "/app/queue"

    fun decide(url: String, origin: String): NavDecision = TODO("TW-1")
}
