// Up-front tests for TW-1 (kiosk-shared-phone §7.11.4 W-D8; §7.11.6 W10), written by the TW-0 skeleton. They fail
// until TW-1 fills PairingUrl. TW-1 must not edit, delete or weaken them.
package com.spacing.xchange.tablet

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class PairingUrlTest {
    private val origin = "https://tablet.xchange.test"
    private val canonical = "$origin/app/queue#pair=ABCDE-FGHJK"

    // --- fromCode: a typed code ---

    @Test
    fun `a code as shown becomes the start page with pair`() {
        assertEquals(canonical, PairingUrl.fromCode("ABCDE-FGHJK", origin))
    }

    @Test
    fun `a code typed lower case, without the dash or with spaces, is the same code`() {
        assertEquals(canonical, PairingUrl.fromCode("abcdefghjk", origin))
        assertEquals(canonical, PairingUrl.fromCode("  abcde fghjk  ", origin))
        assertEquals(canonical, PairingUrl.fromCode("ABCDE - FGHJK", origin))
        assertEquals("$origin/app/queue#pair=01234-56789", PairingUrl.fromCode("0123456789", origin))
        assertEquals("$origin/app/queue#pair=VWXYZ-MNPQR", PairingUrl.fromCode("vwxyzmnpqr", origin))
    }

    @Test
    fun `the url follows the origin it is given`() {
        assertEquals("https://other.test:8443/app/queue#pair=ABCDE-FGHJK", PairingUrl.fromCode("ABCDE-FGHJK", "https://other.test:8443"))
    }

    @Test
    fun `a code of the wrong length is refused`() {
        assertNull(PairingUrl.fromCode("", origin))
        assertNull(PairingUrl.fromCode("   ", origin))
        assertNull(PairingUrl.fromCode("ABCDE-FGHJ", origin))
        assertNull(PairingUrl.fromCode("ABCDE-FGHJKM", origin))
    }

    @Test
    fun `a character outside Crockford base32 is refused`() {
        for (bad in listOf("ABCDE-FGHJI", "ABCDE-FGHJL", "ABCDE-FGHJO", "ABCDE-FGHJU", "ABCDE-FGH#K", "ABCDE-FGH&K", "ABCDE_FGHJK", "ABCDÉ-FGHJK")) {
            assertNull(PairingUrl.fromCode(bad, origin), bad)
        }
    }

    // --- fromScan: the admin panel's QR ---

    @Test
    fun `the admin QR is used unchanged`() {
        val qr = "$origin/app/queue?branch=MAIN#pair=ABCDE-FGHJK"
        assertEquals(qr, PairingUrl.fromScan(qr, origin))
    }

    @Test
    fun `white space around a scan is trimmed`() {
        val qr = "$origin/app/queue?branch=MAIN#pair=ABCDE-FGHJK"
        assertEquals(qr, PairingUrl.fromScan("  $qr\n", origin))
    }

    @Test
    fun `a pair among other fragment keys counts, as URLSearchParams reads it`() {
        val qr = "$origin/app/queue?branch=MAIN#x=1&pair=abcdefghjk"
        assertEquals(qr, PairingUrl.fromScan(qr, origin))
    }

    @Test
    fun `a QR for another origin is refused`() {
        assertNull(PairingUrl.fromScan("https://other.test/app/queue?branch=MAIN#pair=ABCDE-FGHJK", origin))
        assertNull(PairingUrl.fromScan("https://tablet.xchange.test.evil.test/app/queue#pair=ABCDE-FGHJK", origin))
        assertNull(PairingUrl.fromScan("https://tablet.xchange.test:8443/app/queue#pair=ABCDE-FGHJK", origin))
        assertNull(PairingUrl.fromScan("http://tablet.xchange.test/app/queue#pair=ABCDE-FGHJK", origin))
    }

    @Test
    fun `a QR with no pair, or an empty one, is refused`() {
        assertNull(PairingUrl.fromScan("$origin/app/queue?branch=MAIN", origin))
        assertNull(PairingUrl.fromScan("$origin/app/queue?branch=MAIN#", origin))
        assertNull(PairingUrl.fromScan("$origin/app/queue?branch=MAIN#pair=", origin))
        assertNull(PairingUrl.fromScan("$origin/app/queue?branch=MAIN#pairing=ABCDE-FGHJK", origin))
    }

    @Test
    fun `a code in the query instead of the fragment is refused`() {
        assertNull(PairingUrl.fromScan("$origin/app/queue?branch=MAIN&pair=ABCDE-FGHJK", origin))
    }

    @Test
    fun `a QR whose pair is not a valid code is refused`() {
        assertNull(PairingUrl.fromScan("$origin/app/queue?branch=MAIN#pair=ABCDE-FGHJI", origin))
        assertNull(PairingUrl.fromScan("$origin/app/queue?branch=MAIN#pair=ABCDE", origin))
    }

    @Test
    fun `only the queue page pairs`() {
        assertNull(PairingUrl.fromScan("$origin/app/sign-in#pair=ABCDE-FGHJK", origin))
        assertNull(PairingUrl.fromScan("$origin/app/sign-up#pair=ABCDE-FGHJK", origin))
        assertNull(PairingUrl.fromScan("$origin/app/history#pair=ABCDE-FGHJK", origin))
        assertNull(PairingUrl.fromScan("$origin/#pair=ABCDE-FGHJK", origin))
    }

    @Test
    fun `text that is not a URL is refused`() {
        assertNull(PairingUrl.fromScan("ABCDE-FGHJK", origin))
        assertNull(PairingUrl.fromScan("", origin))
        assertNull(PairingUrl.fromScan("not a url #pair=ABCDE-FGHJK", origin))
        assertNull(PairingUrl.fromScan("javascript:location='$origin/app/queue#pair=ABCDE-FGHJK'", origin))
        assertNull(PairingUrl.fromScan("intent://x#Intent;S.pair=ABCDE-FGHJK;end", origin))
    }
}
