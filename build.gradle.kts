plugins {
    alias(libs.plugins.android.application) apply false
}

val rustDir = file("rust")
val jniLibsDir = file("app/src/main/jniLibs")

val rustTargets = mapOf(
    "aarch64-linux-android" to "arm64-v8a",
    "armv7-linux-androideabi" to "armeabi-v7a"
)

fun getAndroidNdkPath(): String {
    val localProps = file("local.properties")
    if (localProps.exists()) {
        val props = java.util.Properties().apply { load(localProps.inputStream()) }
        props.getProperty("ndk.dir")?.let { return it }
    }
    return System.getenv("ANDROID_NDK") 
        ?: System.getenv("ANDROID_NDK_HOME")
        ?: throw GradleException("ANDROID_NDK not found. Set ndk.dir in local.properties or ANDROID_NDK env var.")
}

tasks.register<Exec>("buildRust") {
    description = "Build Rust library for all Android targets (release)"
    group = "rust"
    workingDir = rustDir
    environment("ANDROID_NDK", getAndroidNdkPath())
    
    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a",
        "-t", "armeabi-v7a",
        "--platform", "28",
        "-o", jniLibsDir.absolutePath,
        "build", "--release"
    )
}

tasks.register<Exec>("buildRustDebug") {
    description = "Build Rust library for all Android targets (debug)"
    group = "rust"
    workingDir = rustDir
    environment("ANDROID_NDK", getAndroidNdkPath())
    
    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a",
        "-t", "armeabi-v7a",
        "--platform", "28",
        "-o", jniLibsDir.absolutePath,
        "build"
    )
}

tasks.register<Delete>("cleanRust") {
    description = "Clean Rust build artifacts"
    group = "rust"
    delete(file("rust/target"))
}