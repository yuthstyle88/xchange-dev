// The branch-tablet wrapper app (kiosk-shared-phone §7.11). Android only until the owner confirms an iPad (Q-W1).
//
// Two projects:
// - :core is plain Kotlin/JVM and holds the three pure rules (QueueFlowPolicy, PairingUrl, PairState) and their
//   tests. `./gradlew :core:test` needs a JDK 17+ and nothing else: no Android SDK, no device.
// - :app is the Android glue (TabletActivity, SetupActivity, MeClient). It needs an Android SDK. Without one it is
//   left out of the build with a warning, so anything that names it (`:app:assembleRelease`) fails loudly with
//   "project 'app' not found" instead of passing quietly.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "xchange-tablet"

include(":core")

fun androidSdkDir(): String? {
    val local = settingsDir.resolve("local.properties")
    if (local.isFile) {
        val props = java.util.Properties()
        local.inputStream().use { props.load(it) }
        props.getProperty("sdk.dir")?.let { return it }
    }
    return providers.environmentVariable("ANDROID_HOME").orNull
        ?: providers.environmentVariable("ANDROID_SDK_ROOT").orNull
}

val sdk = androidSdkDir()
if (sdk != null && File(sdk).isDirectory) {
    include(":app")
} else {
    logger.warn(
        "xchange-tablet: no Android SDK (local.properties sdk.dir, ANDROID_HOME, ANDROID_SDK_ROOT) - " +
            "building :core only. :app needs an SDK."
    )
}
