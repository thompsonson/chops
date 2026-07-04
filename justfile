# chops — Android dev recipes
# Usage: just <recipe>

android_home := "{{HOME}}/Android/Sdk"
ndk_version := "27.0.12077973"
apk_path := "/tmp/chops.apk"

# ── Android target check ────────────────────────────────────────────────────

check-android:
    export LIBCLANG_PATH=/usr/lib/llvm-15/lib
    export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="{{android_home}}/ndk/{{ndk_version}}/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android21-clang"
    export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
    export AR_aarch64_linux_android="{{android_home}}/ndk/{{ndk_version}}/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"
    export CMAKE_TOOLCHAIN_FILE="{{android_home}}/ndk/{{ndk_version}}/build/cmake/android.toolchain.cmake"
    export ANDROID_ABI=arm64-v8a
    export ANDROID_PLATFORM=21
    cargo check --target aarch64-linux-android --manifest-path app/src-tauri/Cargo.toml

# ── Emulator setup ───────────────────────────────────────────────────────────

setup-emulator:
    {{android_home}}/cmdline-tools/latest/bin/sdkmanager "emulator" "system-images;android-34;google_apis;arm64-v8a"

create-avd:
    echo no | {{android_home}}/cmdline-tools/latest/bin/avdmanager create avd -n chops-test -k "system-images;android-34;google_apis;arm64-v8a"

# ── Emulator lifecycle ───────────────────────────────────────────────────────

start-emulator:
    {{android_home}}/emulator/emulator -avd chops-test -no-window -no-audio -gpu swiftshader_indirect &

wait-boot:
    {{android_home}}/platform-tools/adb wait-for-device
    while [ -z "$({{android_home}}/platform-tools/adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" ]; do sleep 1; done
    @echo "Booted"

stop-emulator:
    {{android_home}}/platform-tools/adb emu kill

# ── Install & debug ──────────────────────────────────────────────────────────

pull-apk tag="v0.1.0-dev.202607031743":
    gh release download "{{tag}}" --repo thompsonson/chops --pattern '*.apk' --dir /tmp

install-apk:
    {{android_home}}/platform-tools/adb install {{apk_path}}

launch:
    {{android_home}}/platform-tools/adb shell am start -n com.chops.app/.MainActivity

logs:
    {{android_home}}/platform-tools/adb logcat -s chops-app rust:T

crash-log:
    {{android_home}}/platform-tools/adb logcat -d -s chops-app rust:T *:F

# ── All-in-one ───────────────────────────────────────────────────────────────

android-test: setup-emulator create-avd start-emulator wait-boot pull-apk install-apk launch logs
