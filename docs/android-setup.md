# Android Build Setup

Guide for building the chops Tauri app for Android.

## Prerequisites

### JDK 17

```bash
sudo apt install openjdk-17-jdk
```

Verify: `java -version` should show 17.x.

### Android SDK Command-Line Tools

```bash
# Create SDK directory
mkdir -p ~/Android/Sdk/cmdline-tools

# Unzip (adjust path to your download)
unzip ~/Downloads/commandlinetools-linux-14742923_latest.zip -d /tmp/android-tools
mv /tmp/android-tools/cmdline-tools ~/Android/Sdk/cmdline-tools/latest
```

### Install SDK Packages

```bash
export ANDROID_HOME="$HOME/Android/Sdk"
export PATH="$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$PATH"

sdkmanager --licenses  # accept all
sdkmanager "platforms;android-34" "platform-tools" "build-tools;34.0.0" "ndk;27.2.12479018"
```

### Environment Variables

Add to `~/.zshrc` or `~/.bashrc`:

```bash
export JAVA_HOME="/usr/lib/jvm/java-17-openjdk-amd64"
export ANDROID_HOME="$HOME/Android/Sdk"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export NDK_HOME="$ANDROID_HOME/ndk/27.2.12479018"
export PATH="$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$PATH"
```

Reload: `source ~/.zshrc`

### Rust Android Targets

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
```

## Initialize Android Project

```bash
cd app
npx tauri android init
```

This generates `app/src-tauri/gen/android/` with the Gradle project.

### Android Permissions

After init, edit `app/src-tauri/gen/android/app/src/main/AndroidManifest.xml` — add inside `<manifest>`, before `<application>`:

```xml
<uses-permission android:name="android.permission.RECORD_AUDIO" />
<uses-permission android:name="android.permission.INTERNET" />
```

### Runtime Permission Request

Edit `app/src-tauri/gen/android/app/src/main/java/com/chops/app/MainActivity.kt` to request mic permission at startup:

```kotlin
package com.chops.app

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

class MainActivity : TauriActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO)
            != PackageManager.PERMISSION_GRANTED) {
            ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.RECORD_AUDIO), 1)
        }
    }
}
```

## Whisper Model

The app looks for the model in Tauri's `appDataDir`. On Android this is the app's internal storage. Push via adb:

```bash
# Find the app data path (typically /data/data/com.chops.app/)
adb shell run-as com.chops.app mkdir -p files
adb push ggml-base.en.bin /data/local/tmp/
adb shell run-as com.chops.app cp /data/local/tmp/ggml-base.en.bin files/ggml-base.en.bin
```

Alternatively, add a model download feature in the app UI (future work).

## Build

### Dev (connected device)

```bash
cd app
npx tauri android dev
```

### Release APK

```bash
cd app
npx tauri android build
```

Output: `app/src-tauri/gen/android/app/build/outputs/apk/`

## Troubleshooting

### whisper-rs cross-compilation fails (cmake/bindgen)

whisper-rs compiles whisper.cpp from source using cmake. The NDK toolchain must be used:

```bash
# These are usually set automatically by cargo-ndk / tauri, but if cmake fails:
export CMAKE_TOOLCHAIN_FILE="$NDK_HOME/build/cmake/android.toolchain.cmake"
export ANDROID_ABI=arm64-v8a
```

If bindgen can't find `libclang`:

```bash
sudo apt install libclang-dev
export LIBCLANG_PATH="/usr/lib/llvm-14/lib"  # adjust version
```

### Gradle build fails with "SDK not found"

Ensure `ANDROID_HOME` is set and the `local.properties` file in `gen/android/` points to the right SDK path:

```
sdk.dir=/home/you/Android/Sdk
```

### "No toolchains found" for aarch64-linux-android

```bash
rustup target add aarch64-linux-android
```

### cpal/oboe audio issues

cpal uses the `oboe` backend on Android automatically. If audio capture fails, check that `RECORD_AUDIO` permission was granted (Settings > Apps > chops > Permissions).

### Logging not visible

`tracing_subscriber::fmt` outputs to stdout which is invisible on Android. To see logs:

```bash
adb logcat | grep chops
```

For proper Android logging, add `tracing-android` (future improvement).
