// TW-0 skeleton (kiosk-shared-phone §7.11.4 W-D8; §7.11.5; §7.11.6 W10). The bodies are TW-1's.
package com.spacing.xchange.tablet

import java.net.URI

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
    private const val CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
    private const val CODE_LEN = 10

    private fun normalizeCode(code: String): String? {
        val stripped = buildString {
            for (ch in code) {
                if (ch != '-' && !ch.isWhitespace()) {
                    append(ch.uppercaseChar())
                }
            }
        }
        if (stripped.length != CODE_LEN || !stripped.all { it in CROCKFORD }) return null
        return "${stripped.substring(0, 5)}-${stripped.substring(5)}"
    }

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
    fun fromScan(text: String, origin: String): String? {
        val trimmed = text.trim()
        if (QueueFlowPolicy.decide(trimmed, origin) != NavDecision.Allow) return null
        val uri = try {
            URI(trimmed)
        } catch (_: Exception) {
            return null
        }
        if (uri.rawPath != QueueFlowPolicy.START_PATH) return null
        val fragment = uri.rawFragment ?: return null
        val pairValue = fragment
            .split('&')
            .firstNotNullOfOrNull { part ->
                val eq = part.indexOf('=')
                if (eq >= 0 && part.substring(0, eq) == "pair") part.substring(eq + 1) else null
            } ?: return null
        if (normalizeCode(pairValue) == null) return null
        return trimmed
    }

    /**
     * A typed code: `<origin>/app/queue#pair=<CODE>`, where `<CODE>` is the code's 10 characters, upper-cased, as
     * `XXXXX-XXXXX` (the form the admin panel shows). `"abcde fghjk"` ⇒ `<origin>/app/queue#pair=ABCDE-FGHJK`.
     * `null` when the code is not valid.
     */
    fun fromCode(code: String, origin: String): String? {
        val formatted = normalizeCode(code) ?: return null
        return "$origin${QueueFlowPolicy.START_PATH}#pair=$formatted"
    }
}
