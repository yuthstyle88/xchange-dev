// The pure rules of the wrapper (kiosk-shared-phone §7.11.5): no Android types, so the tests run on a plain JVM.
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("org.jetbrains.kotlin.jvm")
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

dependencies {
    // PairState.fromMe reads `me`'s JSON. kotlinx-serialization-json is used as a plain library
    // (Json.parseToJsonElement), with no compiler plugin; it is the same code on the JVM and on Android.
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.9.0")

    testImplementation(kotlin("test"))
    testImplementation(platform("org.junit:junit-bom:5.12.2"))
    testImplementation("org.junit.jupiter:junit-jupiter")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

// The shared cases (W-D7) live once, beside both apps.
val policyCases = rootProject.layout.projectDirectory.file("../policy-cases.json")

tasks.test {
    useJUnitPlatform()
    inputs.file(policyCases).withPropertyName("policyCases")
    systemProperty("xc.policyCases", policyCases.asFile.absolutePath)
    testLogging {
        events("failed")
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.SHORT
    }
}
