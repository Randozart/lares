plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("com.google.protobuf")
}

android {
    namespace = "dev.randozart.lares"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.randozart.lares"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
    }

    // JNA and JNA-platform ship duplicate license metadata.
    packaging {
        resources {
            excludes += setOf("META-INF/AL2.0", "META-INF/LGPL2.1")
        }
    }

    // Protos live at ../proto (symlinked into src/main/proto so the protobuf
    // plugin's default source dir finds them without Kotlin DSL accessors).
}

protobuf {
    protoc {
        artifact = "com.google.protobuf:protoc:4.28.2"
    }
    generateProtoTasks {
        all().forEach { task ->
            task.builtins.create("java")
        }
    }
}

// Contract-first: the shared protobuf contract lives at ../proto. The protobuf
// plugin defaults to src/main/proto, so sync a working copy there before code
// generation. The copy is gitignored; ../proto remains the single source.
val syncProtoContract by tasks.registering(Sync::class) {
    group = "proto"
    description = "Sync the shared protobuf contract into the default proto dir."
    from("../../proto")
    into(layout.projectDirectory.dir("src/main/proto"))
    include("**/*.proto")
}
tasks.named("extractDebugProto") {
    dependsOn(syncProtoContract)
}
tasks.named("extractReleaseProto") {
    dependsOn(syncProtoContract)
}

// --- lares-tracking (Rust, via cargo-ndk + UniFFI) --------------------------

// Resolve the Android NDK root from the environment, falling back to the
// installed NDK 27rc under the standard SDK layout.
val ndkRoot = providers.environmentVariable("ANDROID_NDK_ROOT")
    .orElse(
        providers.environmentVariable("ANDROID_HOME")
            .map { "$it/ndk/android-ndk-r27c" },
    )

val cargoNdkBuild by tasks.registering(Exec::class) {
    group = "lares"
    description = "Cross-compile lares-tracking for arm64-v8a via cargo-ndk."
    dependsOn(syncProtoContract)
    workingDir = rootProject.projectDir.parentFile
    environment("ANDROID_NDK_ROOT", ndkRoot.get())
    commandLine(
        "cargo", "ndk", "-t", "arm64-v8a",
        "-o", project.layout.projectDirectory.dir("src/main/jniLibs").asFile.absolutePath,
        "build", "--release", "-p", "lares-tracking",
    )
}

val generateUniffiKotlin by tasks.registering(Exec::class) {
    group = "lares"
    description = "Generate Kotlin bindings for lares-tracking via uniffi-bindgen."
    dependsOn(cargoNdkBuild)
    workingDir = rootProject.projectDir.parentFile
    val library = project.layout.projectDirectory
        .dir("src/main/jniLibs/arm64-v8a/liblares_tracking.so").asFile.absolutePath
    val outDir = project.layout.buildDirectory.dir("generated/lares/uniffi").get().asFile.absolutePath
    commandLine(
        "cargo", "run", "-p", "lares-tracking", "--bin", "uniffi-bindgen", "--",
        "generate", "--library", library, "--language", "kotlin", "--out-dir", outDir,
    )
}

android {
    sourceSets.getByName("main") {
        java.srcDir("build/generated/lares/uniffi")
    }
}

tasks.named("preBuild") {
    dependsOn(generateUniffiKotlin)
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.10.01")
    implementation(composeBom)
    androidTestImplementation(composeBom)

    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")

    val cameraX = "1.4.1"
    implementation("androidx.camera:camera-core:$cameraX")
    implementation("androidx.camera:camera-camera2:$cameraX")
    implementation("androidx.camera:camera-lifecycle:$cameraX")
    implementation("androidx.camera:camera-view:$cameraX")

    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("com.google.protobuf:protobuf-java:4.28.2")
    implementation("com.google.protobuf:protobuf-java-util:4.28.2")

    // UniFFI-generated Kotlin bindings for lares-tracking use JNA. The @aar
// artifact bundles the Android libjnidispatch.so natives.
    implementation("net.java.dev.jna:jna:5.16.0@aar")

    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    implementation("androidx.work:work-runtime-ktx:2.9.1")

    debugImplementation("androidx.compose.ui:ui-tooling")
    implementation("androidx.compose.ui:ui-tooling-preview")
}
