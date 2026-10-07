plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.pockethle.app"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.pockethle.app"
        minSdk = 24
        targetSdk = 34
        versionCode = 2
        versionName = "0.3.1"

        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a")
        }
    }

    signingConfigs {
        create("release") {
            // Stable signing key so every build upgrades in place instead of
            // demanding an uninstall. Committed for this personal fork; the
            // env vars let CI or a fork override it without editing this file.
            val ksFile = file("../keystore/niobiumhle.keystore")
            storeFile = if (System.getenv("NIOBIUMHLE_KEYSTORE") != null) {
                file(System.getenv("NIOBIUMHLE_KEYSTORE"))
            } else {
                ksFile
            }
            storePassword = System.getenv("NIOBIUMHLE_KEYSTORE_PASS") ?: "niobiumhle-key"
            keyAlias = "niobiumhle"
            keyPassword = System.getenv("NIOBIUMHLE_KEY_PASS") ?: "niobiumhle-key"
        }
    }

    buildTypes {
        debug {
            // CI runners regenerate ~/.android/debug.keystore per run, which
            // broke upgrades between builds. Sign debug with the stable key.
            signingConfig = signingConfigs.getByName("release")
        }
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("release")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.activity:activity-ktx:1.9.0")
    implementation("androidx.fragment:fragment-ktx:1.7.1")
    implementation("androidx.recyclerview:recyclerview:1.3.2")
    implementation("androidx.preference:preference-ktx:1.2.1")
    implementation("androidx.constraintlayout:constraintlayout:2.1.4")
    implementation("com.google.android.material:material:1.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.0")
    implementation("org.json:json:20240303")
}
