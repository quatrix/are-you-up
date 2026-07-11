plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "dev.areyouup"
    compileSdk = 34

    defaultConfig {
        applicationId = "dev.areyouup"
        minSdk = 34
        targetSdk = 34
        versionCode = 1
        versionName = "0.1"
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
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.robolectric:robolectric:4.14.1")
    // Real org.json for JVM unit tests: the mockable android.jar ships
    // non-functional org.json stubs. On the device the framework
    // implementation is used; this artifact never ships in the APK.
    testImplementation("org.json:json:20240303")
    // Real loopback HTTP server for Syncer tests: unit tests compile
    // against android.jar, which excludes JDK-internal modules like
    // com.sun.net.httpserver (jdk.httpserver was never Android API).
    testImplementation("com.squareup.okhttp3:mockwebserver:4.12.0")
}
