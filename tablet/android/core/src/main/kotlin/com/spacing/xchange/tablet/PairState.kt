// TW-0 skeleton (kiosk-shared-phone §7.7 `GET /api/branch-tablet/me`; §7.11.4 W-D8; §7.11.5; §7.11.6 W10, W11).
// The body is TW-1's.
package com.spacing.xchange.tablet

/**
 * What the wrapper learns from one `GET /api/branch-tablet/me` at launch (W-D8).
 * - [Paired] ⇒ open the start page; setup shows "Branch <branchCode> · Tablet <name>" once after pairing.
 * - [Unpaired] ⇒ the setup screen (W10: a new or revoked tablet).
 * - [Unknown] ⇒ open the page as is (W11): setup appears only on an explicit `paired: false`.
 */
sealed interface PairState {
    data class Paired(val branchCode: String, val name: String) : PairState

    data object Unpaired : PairState

    data object Unknown : PairState

    companion object {
        /** The `status` [fromMe] is given when no HTTP answer came back (no network, timeout, TLS failure). */
        const val NO_ANSWER: Int = -1

        /**
         * `status` is the HTTP status, or [NO_ANSWER]; `body` is the response text, or `null` when there was none.
         * - a 2xx status, and a JSON object body with `paired: true` and `tabletId`, `branchCode`, `name` all
         *   non-empty strings ⇒ [Paired] (`branchCode`, `name`). The same test as the page's `readTabletMode`.
         * - a 2xx status, and a JSON object body with `paired: false` (the boolean) ⇒ [Unpaired], whatever else it
         *   holds.
         * - anything else ⇒ [Unknown]: a non-2xx status (a 404 while the server half is not deployed, a 401, a 5xx),
         *   [NO_ANSWER], no body, a body that is not JSON or not an object, `paired` missing or not a boolean, or
         *   `paired: true` with a field missing, empty or not a string. It never throws.
         */
        fun fromMe(status: Int, body: String?): PairState = TODO("TW-1")
    }
}
