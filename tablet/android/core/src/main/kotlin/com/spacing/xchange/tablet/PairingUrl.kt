// TW-0 skeleton (kiosk-shared-phone §7.11.4 W-D8; §7.11.5; §7.11.6 W10). The bodies are TW-1's.
package com.spacing.xchange.tablet

/**
 * The URL the setup screen loads to pair the tablet (W-D8, T-D10). The page then reads `#pair=` and pairs
 * (`pairFromFragment`, `web/src/shared/session/tablet.ts`). Both functions return `null` for anything refused, and
 * never throw. `origin` is as in [QueueFlowPolicy.decide].
 *
 * A pairing code is valid when, with `-` and white space removed and letters upper-cased, it is exactly 10
 * characters of Crockford base32 (`0-9 A-Z` without `I L O U`). That is the server's own rule
 * (`AgentToken::code_hash`, `api/src/infrastructure/device_agents/agent_token.rs`), so a code the server would refuse
 * is refused here first.
 */
object PairingUrl {
    /**
     * A scanned QR (the admin panel's `<origin>/app/queue?branch=<code>#pair=<CODE>`, `BranchTablets.tsx`):
     * the text with surrounding white space trimmed, unchanged otherwise, when all of these hold:
     * - [QueueFlowPolicy.decide] answers [NavDecision.Allow] for it with `origin` (so another origin, `http` and any
     *   other scheme are refused);
     * - its path is exactly `/app/queue` (the only page that pairs);
     * - its fragment, read as `key=value&...` (as the page's `URLSearchParams`), has a `pair` whose value is a valid
     *   code. A code in the query instead of the fragment is refused: T-D10 keeps it out of request lines.
     * Else `null` (W10: "A QR for another origin, or with no `#pair=`, is refused").
     */
    fun fromScan(text: String, origin: String): String? = TODO("TW-1")

    /**
     * A typed code: `<origin>/app/queue#pair=<CODE>`, where `<CODE>` is the code's 10 characters, upper-cased, as
     * `XXXXX-XXXXX` (the form the admin panel shows). `"abcde fghjk"` ⇒ `<origin>/app/queue#pair=ABCDE-FGHJK`.
     * `null` when the code is not valid.
     */
    fun fromCode(code: String, origin: String): String? = TODO("TW-1")
}
