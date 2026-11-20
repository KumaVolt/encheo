// Encheo for Android: the Rust app (libencheo.so, built by scripts/android.sh with cargo-ndk)
// inside a small Java shell for what only Java can do (the music service and lock-screen controls).
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
rootProject.name = "encheo"
include(":app")
