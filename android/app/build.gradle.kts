plugins { id("com.android.application") }
android {
    namespace = "dev.prism.android"
    compileSdk = 37
    defaultConfig {
        applicationId = "dev.prism.android"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0-alpha.1"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }
    buildFeatures { buildConfig = true }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    packaging { jniLibs.useLegacyPackaging = false }
}
dependencies {
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test.uiautomator:uiautomator:2.3.0")
}
