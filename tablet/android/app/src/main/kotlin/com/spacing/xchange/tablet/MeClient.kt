// TW-0 skeleton (kiosk-shared-phone §7.7 `GET /api/branch-tablet/me`; §7.11.4 W-D8; §7.11.5). The body is TW-2's.
package com.spacing.xchange.tablet

import java.net.HttpURLConnection
import java.net.URL

/** One answer of `me`: the HTTP status (or [PairState.NO_ANSWER]) and the body text, as [PairState.fromMe] takes them. */
data class MeResponse(val status: Int, val body: String?)

/**
 * The wrapper's one read of `GET <origin>/api/branch-tablet/me` (T-D6), sent with `cookie`: the WebView's own cookie
 * for `origin` (`CookieManager.getInstance().getCookie(origin)`), which holds the HttpOnly `__Host-xc_tablet` (T-D3).
 * It never throws: a failure to connect or read is `MeResponse(PairState.NO_ANSWER, null)`. It runs off the main
 * thread. It never follows a redirect to another origin.
 */
object MeClient {
    fun get(origin: String, cookie: String?): MeResponse {
        var conn: HttpURLConnection? = null
        return try {
            conn = (URL("$origin/api/branch-tablet/me").openConnection() as HttpURLConnection).apply {
                requestMethod = "GET"
                instanceFollowRedirects = false
                connectTimeout = 5_000
                readTimeout = 5_000
                if (!cookie.isNullOrBlank()) {
                    setRequestProperty("Cookie", cookie)
                }
            }
            val status = conn.responseCode
            val stream = if (status in 200..399) conn.inputStream else conn.errorStream
            val body = stream?.bufferedReader(Charsets.UTF_8)?.use { it.readText() }
            MeResponse(status, body)
        } catch (_: Throwable) {
            MeResponse(PairState.NO_ANSWER, null)
        } finally {
            conn?.disconnect()
        }
    }
}
