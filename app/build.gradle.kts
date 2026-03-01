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
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
}

dependencies { }

tasks.matching { it.name == "mergeDebugJniLibFolders" }.configureEach {
    dependsOn(":buildRustDebug")
}

tasks.matching { it.name == "mergeReleaseJniLibFolders" }.configureEach {
    dependsOn(":buildRust")
}

tasks.matching { it.name == "mergeDebugAssets" || it.name == "mergeReleaseAssets" }.configureEach {
    dependsOn(":buildCustomBootstrap")
    dependsOn(":buildTermuxExecCompat")
}
