// The Android glue of the wrapper (kiosk-shared-phone §7.11.4 W-D1, W-D11; §7.11.5). The rules are in :core.
import java.util.Properties

plugins {
    id("com.android.application")
}

// W-D5: the one origin, per build (gradle.properties `xc.origin`, or -Pxc.origin=...).
val xcOrigin: String = providers.gradleProperty("xc.origin").get()
require(Regex("^https://[a-z0-9.-]+(:[0-9]{1,5})?$").matches(xcOrigin)) {
    "xc.origin must be https://host or https://host:port with no path or trailing slash, got: $xcOrigin"
}

// Release signing, as app/android/app/build.gradle.kts: `tablet/android/key.properties` (storeFile, storePassword,
// keyAlias, keyPassword) when it exists, else the debug key. The release workflow (TW-3) writes that file from the
// four XCHANGE_ANDROID_* secrets, as .github/workflows/app-android.yml does for app/android, and deletes it after.
val keyPropertiesFile = rootProject.file("key.properties")
val keyProperties = Properties()
val hasReleaseSigning = keyPropertiesFile.exists()
if (hasReleaseSigning) {
    keyPropertiesFile.inputStream().use { keyProperties.load(it) }
} else {
    logger.warn("xchange-tablet: tablet/android/key.properties not found - release builds fall back to DEBUG signing.")
}

android {
    namespace = "com.spacing.xchange.tablet"
    compileSdk = 36

    defaultConfig {
        // W-D1: never com.spacing.xchange, so it never replaces the queue machine app.
        applicationId = "com.spacing.xchange.tablet"
        // onRenderProcessGone (W9) needs API 26.
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "1.0.0"
        buildConfigField("String", "XC_ORIGIN", "\"$xcOrigin\"")
    }

    buildFeatures {
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = file(keyProperties.getProperty("storeFile"))
                storePassword = keyProperties.getProperty("storePassword")
                keyAlias = keyProperties.getProperty("keyAlias")
                keyPassword = keyProperties.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = if (hasReleaseSigning) {
                signingConfigs.getByName("release")
            } else {
                signingConfigs.getByName("debug")
            }
        }
    }
}

dependencies {
    implementation(project(":core"))
    // W-D8: the setup screen scans the admin QR with Google's code scanner, which needs no camera permission.
    implementation("com.google.android.gms:play-services-code-scanner:16.1.0")
}
