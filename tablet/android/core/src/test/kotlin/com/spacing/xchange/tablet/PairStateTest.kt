// Up-front tests for TW-1 (kiosk-shared-phone §7.7 `me`; §7.11.4 W-D8; §7.11.6 W10, W11 and its "Tests" list:
// paired true, paired false, a missing field, non-JSON, 404 and 5xx), written by the TW-0 skeleton. They fail until
// TW-1 fills PairState.fromMe. TW-1 must not edit, delete or weaken them.
package com.spacing.xchange.tablet

import kotlin.test.Test
import kotlin.test.assertEquals

class PairStateTest {
    private val paired = """{"paired":true,"tabletId":"7d1c3a52-0000-4000-8000-000000000001","branchCode":"MAIN","name":"Front-1"}"""

    @Test
    fun `paired true with its three fields is Paired`() {
        assertEquals(PairState.Paired(branchCode = "MAIN", name = "Front-1"), PairState.fromMe(200, paired))
    }

    @Test
    fun `extra fields do not matter`() {
        val body = """{"paired":true,"tabletId":"t-1","branchCode":"SILOM","name":"Front 2","branchName":"สาขาสีลม"}"""
        assertEquals(PairState.Paired("SILOM", "Front 2"), PairState.fromMe(200, body))
        assertEquals(PairState.Unpaired, PairState.fromMe(200, """{"paired":false,"tabletId":"t-1"}"""))
    }

    @Test
    fun `paired false is Unpaired, the only answer that opens setup`() {
        assertEquals(PairState.Unpaired, PairState.fromMe(200, """{"paired":false}"""))
        assertEquals(PairState.Unpaired, PairState.fromMe(200, """ { "paired" : false } """))
    }

    @Test
    fun `paired true with a field missing is Unknown`() {
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":true,"tabletId":"t-1","branchCode":"MAIN"}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":true,"tabletId":"t-1","name":"Front-1"}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":true,"branchCode":"MAIN","name":"Front-1"}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":true}"""))
    }

    @Test
    fun `paired true with a field empty, null or not a string is Unknown`() {
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":true,"tabletId":"t-1","branchCode":"","name":"Front-1"}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":true,"tabletId":"t-1","branchCode":"MAIN","name":""}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":true,"tabletId":"","branchCode":"MAIN","name":"Front-1"}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":true,"tabletId":"t-1","branchCode":null,"name":"Front-1"}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":true,"tabletId":"t-1","branchCode":7,"name":"Front-1"}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":true,"tabletId":"t-1","branchCode":"MAIN","name":["Front-1"]}"""))
    }

    @Test
    fun `paired missing or not a boolean is Unknown`() {
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":"false"}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":0}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":null}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":"true","tabletId":"t-1","branchCode":"MAIN","name":"Front-1"}"""))
    }

    @Test
    fun `a body that is not a JSON object is Unknown`() {
        assertEquals(PairState.Unknown, PairState.fromMe(200, "not json"))
        assertEquals(PairState.Unknown, PairState.fromMe(200, "<html>502 Bad Gateway</html>"))
        assertEquals(PairState.Unknown, PairState.fromMe(200, ""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, "[]"))
        assertEquals(PairState.Unknown, PairState.fromMe(200, "false"))
        assertEquals(PairState.Unknown, PairState.fromMe(200, "null"))
        assertEquals(PairState.Unknown, PairState.fromMe(200, """{"paired":false"""))
        assertEquals(PairState.Unknown, PairState.fromMe(200, null))
    }

    @Test
    fun `a 404 is Unknown even with a paired false body, so an undeployed server never sends the tablet to setup`() {
        assertEquals(PairState.Unknown, PairState.fromMe(404, """{"paired":false}"""))
        assertEquals(PairState.Unknown, PairState.fromMe(404, null))
    }

    @Test
    fun `any non-2xx answer is Unknown`() {
        for (status in listOf(301, 302, 400, 401, 403, 429, 500, 502, 503, 504)) {
            assertEquals(PairState.Unknown, PairState.fromMe(status, """{"paired":false}"""), "status $status")
            assertEquals(PairState.Unknown, PairState.fromMe(status, paired), "status $status")
        }
    }

    @Test
    fun `no answer at all is Unknown`() {
        assertEquals(PairState.Unknown, PairState.fromMe(PairState.NO_ANSWER, null))
        assertEquals(PairState.Unknown, PairState.fromMe(0, null))
    }
}
