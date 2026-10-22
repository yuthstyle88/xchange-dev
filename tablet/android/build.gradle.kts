// Plugin versions for the whole build, applied by each project (kiosk-shared-phone §7.11.5, build row).
// AGP 9 compiles :app's Kotlin itself (built-in Kotlin), on the Kotlin Gradle plugin declared here, so both
// projects build with one Kotlin version.
plugins {
    id("com.android.application") version "9.0.1" apply false
    id("org.jetbrains.kotlin.jvm") version "2.3.20" apply false
}
