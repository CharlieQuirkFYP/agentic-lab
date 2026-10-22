#!/usr/bin/env bash
set -euo pipefail

# Isolated target directories prevent test cfgs/features leaking into production.
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
VARIANT=${1:-whisper}
case "$VARIANT" in whisper|disabled|test) ;; *) echo "Usage: bash scripts/build-ios.sh [whisper|disabled|test]" >&2; exit 2 ;; esac
[[ $(uname -m) == arm64 ]] || { echo "An Apple Silicon Mac is required." >&2; exit 1; }
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
    shasum -a 256 Cargo.lock include/pheme_va.h
} > "$OUT/build-metadata.txt"

rustup target add aarch64-apple-ios aarch64-apple-ios-sim
rustup component add llvm-tools-preview
# Apple nm may not understand LLVM bitcode bundled with Rust's standard library.
# Inspect with the tools from the exact Rust toolchain that builds this archive.
RUST_HOST=$(rustc -vV | sed -n 's/^host: //p')
LLVM_NM="$(rustc --print sysroot)/lib/rustlib/$RUST_HOST/bin/llvm-nm"
[[ -x "$LLVM_NM" ]] || { echo "Missing Rust llvm-nm: $LLVM_NM" >&2; exit 1; }
"$LLVM_NM" --version >> "$OUT/build-metadata.txt"

# Resolve/download once, then keep both slice builds offline and lockfile-bound.
# Cargo's bounded network retries apply here; persistent DNS failures still fail.
cargo fetch --locked --target aarch64-apple-ios --target aarch64-apple-ios-sim
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
    cargo build --frozen -p ffi --lib --release "${FEATURES[@]}" --target "$TARGET"
    "$LLVM_NM" --extern-only --defined-only "$CARGO_TARGET_DIR/$TARGET/release/libffi.a" > "$OUT/$TARGET-symbols.txt"
    if ! grep -q '_pheme_va_abi_version$' "$OUT/$TARGET-symbols.txt"; then
        echo "Stable ABI entry point missing from $TARGET." >&2; exit 1
    fi
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
shasum -a 256 "$CARGO_TARGET_DIR"/*/release/libffi.a >> "$OUT/build-metadata.txt"
echo "Packaged $OUT/PhemeVA.xcframework"
