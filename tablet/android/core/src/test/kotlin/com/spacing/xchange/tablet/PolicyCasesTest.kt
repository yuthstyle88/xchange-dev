// Up-front tests for TW-1 (kiosk-shared-phone §7.11.6 W5-W7; W-D7), written by the TW-0 skeleton. They fail until
// TW-1 fills QueueFlowPolicy.decide. TW-1 must not edit, delete or weaken them; a new case goes into
// tablet/policy-cases.json, which the iPad app's tests read too.
package com.spacing.xchange.tablet

import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.jupiter.api.DynamicTest
import org.junit.jupiter.api.TestFactory

class PolicyCasesTest {
    private data class Case(val url: String, val expect: NavDecision, val why: String)

    private val file: File =
        File(requireNotNull(System.getProperty("xc.policyCases")) { "xc.policyCases is set by core/build.gradle.kts" })

    private val root: JsonObject = Json.parseToJsonElement(file.readText()).jsonObject

    private val origin: String = root.getValue("origin").jsonPrimitive.content

    private val cases: List<Case> =
        root.getValue("cases").jsonArray.map { element ->
            val o = element.jsonObject
            Case(
                url = o.getValue("url").jsonPrimitive.content,
                expect = when (val e = o.getValue("expect").jsonPrimitive.content) {
                    "allow" -> NavDecision.Allow
                    "start" -> NavDecision.StartPage
                    "ignore" -> NavDecision.Ignore
                    else -> error("unknown expect '$e' in ${file.name}")
                },
                why = o["why"]?.jsonPrimitive?.content ?: "",
            )
        }

    @Test
    fun `the cases file covers every decision and every W-D5 scheme`() {
        for (d in NavDecision.entries) {
            assertTrue(cases.any { it.expect == d }, "no case expects $d")
        }
        for (scheme in listOf("http:", "intent:", "tel:", "mailto:", "blob:", "data:", "file:", "javascript:")) {
            assertTrue(cases.any { it.url.startsWith(scheme) && it.expect == NavDecision.Ignore }, "no ignored $scheme case")
        }
        for (path in QueueFlowPolicy.FLOW_PATHS) {
            assertTrue(cases.any { it.url == origin + path && it.expect == NavDecision.Allow }, "no allowed $path case")
        }
        assertTrue(
            cases.any { it.url.startsWith(origin + QueueFlowPolicy.START_PATH) && "#pair=" in it.url && it.expect == NavDecision.Allow },
            "no allowed #pair= case on the start page",
        )
        assertEquals(cases.size, cases.map { it.url }.toSet().size, "a URL appears twice in ${file.name}")
    }

    @TestFactory
    fun `every case in policy-cases json`(): List<DynamicTest> =
        cases.map { c ->
            DynamicTest.dynamicTest("${c.expect} <- '${c.url}' (${c.why})") {
                assertEquals(c.expect, QueueFlowPolicy.decide(c.url, origin), "${c.url}: ${c.why}")
            }
        }

    @Test
    fun `the start page itself is allowed`() {
        assertEquals(NavDecision.Allow, QueueFlowPolicy.decide(origin + QueueFlowPolicy.START_PATH, origin))
    }

    @Test
    fun `the rule follows the origin it is given, not the one in the cases file`() {
        val other = "https://other.test:8443"
        assertEquals(NavDecision.Allow, QueueFlowPolicy.decide("$other/app/queue?branch=MAIN", other))
        assertEquals(NavDecision.StartPage, QueueFlowPolicy.decide("$other/app/history", other))
        assertEquals(NavDecision.Ignore, QueueFlowPolicy.decide("https://other.test/app/queue", other))
        assertEquals(NavDecision.Ignore, QueueFlowPolicy.decide("$origin/app/queue", other))
    }
}
