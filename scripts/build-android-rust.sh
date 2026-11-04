#!/usr/bin/env bash
# Builds nik8-ffi for Android and generates its Kotlin bindings into the
# nik8-native Expo module. Usage: scripts/build-android-rust.sh [--debug] [abi...]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODULE_ANDROID="$ROOT/nik8-mobile/modules/nik8-native/android"
JNI_LIBS="$MODULE_ANDROID/src/main/jniLibs"
KOTLIN_OUT="$MODULE_ANDROID/src/main/java"

# `mobile` = release with panic = "unwind" (see workspace Cargo.toml).
PROFILE=mobile
PROFILE_FLAG=(--profile mobile)
if [[ "${1:-}" == "--debug" ]]; then
  PROFILE=debug
  PROFILE_FLAG=()
  shift
fi

ABIS=("$@")
if [[ ${#ABIS[@]} -eq 0 ]]; then
  ABIS=(arm64-v8a x86_64)
fi

TARGET_FLAGS=()
for abi in "${ABIS[@]}"; do
  TARGET_FLAGS+=(-t "$abi")
done

cd "$ROOT"

# 16 KB page alignment, required for Android 15+ devices.
export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384"

cargo ndk "${TARGET_FLAGS[@]}" -o "$JNI_LIBS" build -p nik8_ffi "${PROFILE_FLAG[@]}"

# Bindings come from the library's embedded metadata; the host build is enough.
unset RUSTFLAGS
cargo build -p nik8_ffi
cargo run -q -p nik8_ffi --bin uniffi-bindgen -- generate \
  --library "$ROOT/target/debug/libnik8_ffi.so" \
  --language kotlin \
  --no-format \
  --out-dir "$KOTLIN_OUT"

echo "nik8-ffi ($PROFILE) → $JNI_LIBS"
ls -la "$JNI_LIBS"/*/libnik8_ffi.so
