plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "org.retifs.desktop"
    compileSdk = 34

    defaultConfig {
        applicationId = "org.retifs.desktop"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.0.1"
        // The daemons and the core are built for 64-bit ARM phones (and x86_64 emulators).
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildFeatures { compose = true }

    buildTypes {
        // Shrunk (R8 keeps the native methods the JNI library implements).
        // Signed with the debug key for now: for testers, not a store.
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    // retifsd runs as a program from the native library folder (Android
    // lets an app execute its native libraries, not files it writes): it
    // must be extracted there, not left in the APK.
    packaging { jniLibs { useLegacyPackaging = true } }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material.icons)
}
