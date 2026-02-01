plugins {
    alias(libs.plugins.android.application)
}

android {
    namespace = "com.mynk8.gui_engine"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.mynk8.gui_engine"
        minSdk = 24
        targetSdk = 36
        versionCode = 1
        versionName = "1.0"

        ndk {
            abiFilters += "arm64-v8a"
            abiFilters += "armeabi-v7a"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
}

dependencies { }

tasks.whenTaskAdded {
    if (name == "mergeDebugJniLibFolders") {
        dependsOn(":buildRustDebug")
    }
    if (name == "mergeReleaseJniLibFolders") {
        dependsOn(":buildRust")
    }
}