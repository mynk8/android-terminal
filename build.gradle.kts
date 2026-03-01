plugins {
    alias(libs.plugins.android.application) apply false
}

val rustDir = file("rust")
val jniLibsDir = file("app/src/main/jniLibs")
val bootstrapAsset = file("app/src/main/assets/bootstrap-aarch64.zip")
val bootstrapUpstream = file("build/bootstrap/bootstrap-aarch64-upstream.zip")

val rustTargets = mapOf(
    "aarch64-linux-android" to "arm64-v8a"
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

fun getNdkToolchainBinDir(): String {
    val os = System.getProperty("os.name").lowercase()
    val arch = System.getProperty("os.arch").lowercase()
    val hostTag = when {
        os.contains("linux") && (arch.contains("aarch64") || arch.contains("arm64")) -> "linux-aarch64"
        os.contains("linux") -> "linux-x86_64"
        os.contains("mac") && (arch.contains("aarch64") || arch.contains("arm64")) -> "darwin-arm64"
        os.contains("mac") -> "darwin-x86_64"
        os.contains("win") -> "windows-x86_64"
        else -> throw GradleException("Unsupported host: os=$os arch=$arch")
    }
    return "${getAndroidNdkPath()}/toolchains/llvm/prebuilt/$hostTag/bin"
}

fun sha256(file: File): String {
    val digest = java.security.MessageDigest.getInstance("SHA-256")
    file.inputStream().use { input ->
        val buffer = ByteArray(8192)
        while (true) {
            val read = input.read(buffer)
            if (read <= 0) break
            digest.update(buffer, 0, read)
        }
    }
    return digest.digest().joinToString("") { "%02x".format(it) }
}

fun looksLikeText(data: ByteArray): Boolean {
    if (data.isEmpty()) return true
    if (data.any { it == 0.toByte() }) return false
    var suspicious = 0
    for (b in data) {
        val u = b.toInt() and 0xff
        if (u < 0x09 || (u in 0x0e..0x1f)) suspicious++
    }
    return suspicious * 10 < data.size
}

fun sanitizeBootstrapEntry(
    data: ByteArray,
    replacements: List<Pair<String, String>>,
): ByteArray {
    if (!looksLikeText(data)) return data
    var text = data.toString(Charsets.UTF_8)
    for ((oldValue, newValue) in replacements) {
        text = text.replace(oldValue, newValue)
    }
    return text.toByteArray(Charsets.UTF_8)
}

tasks.register<Exec>("buildRust") {
    description = "Build Rust library for Android arm64 (release)"
    group = "rust"
    workingDir = rustDir
    environment("ANDROID_NDK", getAndroidNdkPath())
    
    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a",
        "--platform", "28",
        "-o", jniLibsDir.absolutePath,
        "build", "--release"
    )
}

tasks.register<Exec>("buildTermuxExecCompat") {
    description = "Build package-agnostic termux-exec compatibility preload library"
    group = "bootstrap"

    val src = file("termux-exec-compat/termux_exec_compat.c")
    val out = file("app/src/main/assets/libtermux-exec.so")
    inputs.file(src)
    outputs.file(out)

    doFirst {
        out.parentFile.mkdirs()
    }

    commandLine(
        "${getNdkToolchainBinDir()}/aarch64-linux-android28-clang",
        "-shared",
        "-fPIC",
        "-O2",
        "-Wall",
        "-Wextra",
        "-Werror",
        src.absolutePath,
        "-o",
        out.absolutePath,
        "-ldl"
    )
}

tasks.register<DefaultTask>("downloadBootstrap") {
    description = "Download Termux bootstrap (apt-android-7)"
    group = "bootstrap"

    doLast {
        val version = "2022.04.28-r5+apt-android-7"
        val url = "https://github.com/termux/termux-packages/releases/download/bootstrap-${version}/bootstrap-aarch64.zip"
        val checksum = "4a51a7eb209fe82efc24d52e3cccc13165f27377290687cb82038cbd8e948430"
        val dest = bootstrapUpstream
        val legacyAsset = bootstrapAsset

        if (dest.exists()) {
            val actual = sha256(dest)
            if (actual == checksum) {
                return@doLast
            }
            dest.delete()
        }

        if (legacyAsset.exists()) {
            val actual = sha256(legacyAsset)
            if (actual == checksum) {
                dest.parentFile.mkdirs()
                legacyAsset.copyTo(dest, overwrite = true)
                return@doLast
            }
        }

        dest.parentFile.mkdirs()

        val digest = java.security.MessageDigest.getInstance("SHA-256")
        val connection = java.net.URL(url).openConnection()
        connection.getInputStream().use { input ->
            java.security.DigestInputStream(input, digest).use { digestStream ->
                dest.outputStream().use { output ->
                    val buffer = ByteArray(8192)
                    while (true) {
                        val read = digestStream.read(buffer)
                        if (read <= 0) break
                        output.write(buffer, 0, read)
                    }
                }
            }
        }

        val actual = digest.digest().joinToString("") { "%02x".format(it) }
        if (actual != checksum) {
            dest.delete()
            throw GradleException("Wrong checksum for $url: expected $checksum, actual $actual")
        }
    }
}

tasks.register<DefaultTask>("buildCustomBootstrap") {
    description = "Build sanitized bootstrap for this package namespace"
    group = "bootstrap"
    dependsOn("downloadBootstrap")
    inputs.file(bootstrapUpstream)
    outputs.file(bootstrapAsset)

    doLast {
        val appId = "com.mynk8.gui_engine"
        val filesRoot = "/data/user/0/$appId/files"
        val cacheRoot = "/data/user/0/$appId/cache"
        val replacements = listOf(
            "packages-cf.termux.org" to "packages-cf.termux.dev",
            "packages.termux.org" to "packages.termux.dev",
            "/data/data/com.termux/files/usr" to "$filesRoot/prefix",
            "/data/user/0/com.termux/files/usr" to "$filesRoot/prefix",
            "/data/data/com.termux/files/home" to "$filesRoot/home",
            "/data/user/0/com.termux/files/home" to "$filesRoot/home",
            "/data/data/com.termux/cache" to cacheRoot,
            "/data/user/0/com.termux/cache" to cacheRoot,
            "/data/data/com.termux/files" to filesRoot,
            "/data/user/0/com.termux/files" to filesRoot,
            "/data/data/com.termux/" to "/data/user/0/$appId/",
            "/data/user/0/com.termux/" to "/data/user/0/$appId/",
        )

        val tmp = file("${bootstrapAsset.absolutePath}.tmp")
        tmp.parentFile.mkdirs()

        java.util.zip.ZipFile(bootstrapUpstream).use { zipIn ->
            java.util.zip.ZipOutputStream(tmp.outputStream()).use { zipOut ->
                val entries = zipIn.entries()
                while (entries.hasMoreElements()) {
                    val entry = entries.nextElement()
                    val outEntry = java.util.zip.ZipEntry(entry.name)
                    outEntry.time = entry.time
                    outEntry.comment = entry.comment
                    outEntry.extra = entry.extra
                    zipOut.putNextEntry(outEntry)
                    if (!entry.isDirectory) {
                        val bytes = zipIn.getInputStream(entry).readBytes()
                        val sanitized = sanitizeBootstrapEntry(bytes, replacements)
                        zipOut.write(sanitized)
                    }
                    zipOut.closeEntry()
                }
            }
        }

        bootstrapAsset.parentFile.mkdirs()
        tmp.copyTo(bootstrapAsset, overwrite = true)
        tmp.delete()

        val forbidden = listOf(
            "/data/data/com.termux",
            "/data/user/0/com.termux"
        )
        val leaks = mutableListOf<String>()
        java.util.zip.ZipFile(bootstrapAsset).use { zip ->
            val entries = zip.entries()
            while (entries.hasMoreElements()) {
                val entry = entries.nextElement()
                if (entry.isDirectory) continue
                val data = zip.getInputStream(entry).readBytes()
                if (!looksLikeText(data)) continue
                val text = data.toString(Charsets.UTF_8)
                if (forbidden.any { text.contains(it) }) {
                    leaks.add(entry.name)
                    if (leaks.size >= 20) break
                }
            }
        }
        if (leaks.isNotEmpty()) {
            throw GradleException(
                "Custom bootstrap still contains hardcoded com.termux prefixes in: " +
                    leaks.joinToString(", ")
            )
        }
    }
}

tasks.register<Exec>("buildRustDebug") {
    description = "Build Rust library for Android arm64 (debug)"
    group = "rust"
    workingDir = rustDir
    environment("ANDROID_NDK", getAndroidNdkPath())
    
    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a",
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
