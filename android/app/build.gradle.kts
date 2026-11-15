plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

val versionNameStr = "0.1.0"

android {
    namespace = "dev.ccontrol.app"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.ccontrol.app"
        minSdk = 29
        targetSdk = 37
        versionCode = 1
        versionName = versionNameStr
    }

    signingConfigs {
        // Release signing only when CI provides secrets; keystores are never committed.
        val ksPath = System.getenv("CCONTROL_KEYSTORE")
        if (ksPath != null && file(ksPath).exists()) create("release") {
            storeFile = file(ksPath)
            storePassword = System.getenv("CCONTROL_KEYSTORE_PASSWORD")
            keyAlias = System.getenv("CCONTROL_KEY_ALIAS")
            keyPassword = System.getenv("CCONTROL_KEY_PASSWORD")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release")
        }
    }

    splits {
        abi {
            isEnable = false // Pure Kotlin app: one APK runs on every ABI (arm64 included).
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildFeatures { compose = true }
}

dependencies {
    implementation(project(":core"))
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.process)
    implementation(libs.androidx.navigation.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)
    implementation(libs.play.services.code.scanner)
    debugImplementation(libs.compose.ui.tooling)
    testImplementation(libs.junit)
}
