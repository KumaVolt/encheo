plugins {
    id("com.android.application")
}

// The version comes from the workspace's Cargo.toml, like every other platform's.
val cargoVersion: String = Regex("""(?m)^version = "(.+)"""")
    .find(rootDir.resolve("../../../Cargo.toml").readText())!!.groupValues[1]
val versionParts = cargoVersion.split(".").map { it.takeWhile(Char::isDigit).toIntOrNull() ?: 0 }

android {
    namespace = "app.encheo"
    compileSdk = 36
    // The NDK strips debug symbols from libencheo.so when packaging (a debug build's is ~450 MB).
    System.getenv("ANDROID_NDK_HOME")?.let { home ->
        ndkPath = home
        ndkVersion = Regex("""Pkg\.Revision\s*=\s*(\S+)""")
            .find(file("$home/source.properties").readText())!!.groupValues[1]
    }

    defaultConfig {
        applicationId = "app.encheo"
        minSdk = 26
        // Google Play requires new apps and updates to target Android 16 (API 36).
        targetSdk = 36
        versionName = cargoVersion
        // cargo-apk's numbering, which 0.4 and earlier shipped with (0.4.1 -> 0x01000401), so
        // new versions install over them as updates.
        versionCode = (1 shl 24) + (versionParts[0] shl 16) + (versionParts[1] shl 8) + versionParts[2]
        ndk { abiFilters += listOf("arm64-v8a") }
    }

    // Release signing from the environment (CI); without it, release builds are unsigned.
    val keystore = System.getenv("ENCHEO_KEYSTORE")
    signingConfigs {
        if (keystore != null) {
            create("release") {
                storeFile = file(keystore)
                storePassword = System.getenv("ENCHEO_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ENCHEO_KEY_ALIAS")
                keyPassword = System.getenv("ENCHEO_KEY_PASSWORD")
            }
        }
    }
    buildTypes {
        release {
            isMinifyEnabled = false
            if (keystore != null) signingConfig = signingConfigs.getByName("release")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
