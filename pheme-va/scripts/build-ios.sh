#!/usr/bin/env bash
set -euo pipefail

# Isolated target directories prevent test cfgs/features leaking into production.
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
VARIANT=${1:-whisper}
case "$VARIANT" in whisper|disabled|test) ;; *) echo "Usage: bash scripts/build-ios.sh [whisper|disabled|test]" >&2; exit 2 ;; esac
[[ $(uname -m) == arm64 ]] || { echo "An Apple Silicon Mac is required." >&2; exit 1; }
xcodebuild -version
export IPHONEOS_DEPLOYMENT_TARGET=17.0
export RUSTUP_TOOLCHAIN=${PHEME_IOS_RUST_TOOLCHAIN:-1.99.0}
export CARGO_TARGET_DIR="$ROOT/target/ios/$VARIANT/cargo"
OUT="$ROOT/target/ios/$VARIANT"
mkdir -p "$OUT/headers"
cp "$ROOT/include/pheme_va.h" "$OUT/headers/"
cp "$ROOT/include/module.modulemap" "$OUT/headers/"
FEATURES=(--no-default-features)
# Do not inherit test hooks or an unrelated caller's target flags.
unset CARGO_ENCODED_RUSTFLAGS
export RUSTFLAGS=""
if [[ "$VARIANT" == whisper ]]; then FEATURES+=(--features whisper); fi
if [[ "$VARIANT" == test ]]; then
    export RUSTFLAGS="--cfg pheme_ffi_test"
    printf '\nPhemeVaHandle *pheme_va_test_new(void);\nPhemeVaHandle *pheme_va_test_create(void);\n' >> "$OUT/headers/pheme_va.h"
fi

cd "$ROOT"
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
for TARGET in aarch64-apple-ios aarch64-apple-ios-sim; do
    SDK=iphoneos
    CLANG_TARGET=arm64-apple-ios17.0
    if [[ "$TARGET" == aarch64-apple-ios-sim ]]; then
        SDK=iphonesimulator
        CLANG_TARGET=arm64-apple-ios17.0-simulator
    fi
    export SDKROOT
    SDKROOT=$(xcrun --sdk "$SDK" --show-sdk-path)
    export CMAKE_SYSTEM_NAME=iOS CMAKE_SYSTEM_PROCESSOR=arm64
    export CMAKE_OSX_ARCHITECTURES=arm64 CMAKE_OSX_SYSROOT="$SDKROOT"
    export CMAKE_OSX_DEPLOYMENT_TARGET="$IPHONEOS_DEPLOYMENT_TARGET"
    export GGML_NATIVE=OFF GGML_CPU_ARM_ARCH=armv8-a GGML_ACCELERATE=ON
    # whisper-rs-sys built on macOS links ggml-blas, including when targeting iOS.
    export GGML_BLAS=ON GGML_BLAS_VENDOR=Apple GGML_METAL=OFF WHISPER_COREML=OFF
    export BINDGEN_EXTRA_CLANG_ARGS="-target $CLANG_TARGET -isysroot $SDKROOT"
    cargo build --locked -p ffi --lib --release "${FEATURES[@]}" --target "$TARGET"
    xcrun nm -gU "$CARGO_TARGET_DIR/$TARGET/release/libffi.a" > "$OUT/$TARGET-symbols.txt"
    if [[ "$VARIANT" != test ]] && grep -q '_pheme_va_test_' "$OUT/$TARGET-symbols.txt"; then
        echo "Test hooks leaked into a production library." >&2; exit 1
    fi
    if [[ "$VARIANT" == disabled ]] && grep -q '_pheme_va_whisper_new' "$OUT/$TARGET-symbols.txt"; then
        echo "Legacy Whisper symbols unexpectedly present in disabled build." >&2; exit 1
    fi
done

# Only remove this script's generated package; source/model directories are untouched.
if [[ -d "$OUT/PhemeVA.xcframework" ]]; then
    rm -rf "$OUT/PhemeVA.xcframework"
fi
xcodebuild -create-xcframework \
    -library "$CARGO_TARGET_DIR/aarch64-apple-ios/release/libffi.a" -headers "$OUT/headers" \
    -library "$CARGO_TARGET_DIR/aarch64-apple-ios-sim/release/libffi.a" -headers "$OUT/headers" \
    -output "$OUT/PhemeVA.xcframework"
{
    printf 'variant=%s\nminimum_ios=%s\n' "$VARIANT" "$IPHONEOS_DEPLOYMENT_TARGET"
    printf 'features=%s\nrustflags=%s\nggml_native=OFF\nggml_cpu_arm_arch=armv8-a\n' "${FEATURES[*]}" "$RUSTFLAGS"
    git rev-parse HEAD
    rustc --version --verbose
    cargo --version
    xcodebuild -version
    xcrun swift --version
    xcrun --sdk iphoneos --show-sdk-version
    xcrun --sdk iphonesimulator --show-sdk-version
    xcodegen --version
    cmake --version
    shasum -a 256 Cargo.lock include/pheme_va.h "$CARGO_TARGET_DIR"/*/release/libffi.a
} > "$OUT/build-metadata.txt"
echo "Packaged $OUT/PhemeVA.xcframework"
