#!/usr/bin/env bash
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
[[ $(uname -m) == arm64 ]] || { echo "An Apple Silicon Mac is required." >&2; exit 1; }
export RUSTUP_TOOLCHAIN=${PHEME_IOS_RUST_TOOLCHAIN:-1.99.0}
export CARGO_TARGET_DIR="$ROOT/target/ios/reply/cargo"
export IPHONEOS_DEPLOYMENT_TARGET=17.0
export RUSTFLAGS=""
unset CARGO_ENCODED_RUSTFLAGS
OUT="$ROOT/target/ios/reply"
mkdir -p "$OUT"
cd "$ROOT"
rustup run "$RUSTUP_TOOLCHAIN" rustc --version
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
    export CMAKE_OSX_DEPLOYMENT_TARGET=17.0
    export GGML_NATIVE=OFF GGML_CPU_ARM_ARCH=armv8-a GGML_ACCELERATE=ON
    export GGML_METAL=OFF GGML_OPENMP=OFF LLAMA_BUILD_SHARED_LIBS=0
    export BINDGEN_EXTRA_CLANG_ARGS="-target $CLANG_TARGET -isysroot $SDKROOT"
    cargo build --frozen -p reply-ffi --lib --release --target "$TARGET"
    FRAMEWORK="$OUT/$TARGET/PhemeReasoning.framework"
    mkdir -p "$FRAMEWORK/Headers" "$FRAMEWORK/Modules"
    cp "$CARGO_TARGET_DIR/$TARGET/release/libreply_ffi.dylib" "$FRAMEWORK/PhemeReasoning"
    cp "$ROOT/include/pheme_reply.h" "$FRAMEWORK/Headers/"
    xcrun install_name_tool -id '@rpath/PhemeReasoning.framework/PhemeReasoning' "$FRAMEWORK/PhemeReasoning"
    cat > "$FRAMEWORK/Modules/module.modulemap" <<'MODULE'
framework module PhemeReasoning {
    umbrella header "pheme_reply.h"
    export *
}
MODULE
    cat > "$FRAMEWORK/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>org.agenticlab.PhemeReasoning</string>
<key>CFBundleExecutable</key><string>PhemeReasoning</string>
<key>CFBundleName</key><string>PhemeReasoning</string>
<key>CFBundlePackageType</key><string>FMWK</string>
<key>CFBundleShortVersionString</key><string>1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>MinimumOSVersion</key><string>17.0</string>
</dict></plist>
PLIST
    xcrun nm -gU "$FRAMEWORK/PhemeReasoning" > "$OUT/$TARGET-exports.txt"
    grep -q '_pheme_reply_create$' "$OUT/$TARGET-exports.txt"
    # The dynamic image must not export its private GGML/llama implementation.
    if grep -Eq ' (_ggml_|_gguf_|_llama_)' "$OUT/$TARGET-exports.txt"; then
        echo "Reasoning framework leaked native symbols." >&2; exit 1
    fi
    xcrun otool -L "$FRAMEWORK/PhemeReasoning" > "$OUT/$TARGET-dependencies.txt"
    shasum -a 256 "$FRAMEWORK/PhemeReasoning" > "$OUT/$TARGET-sha256.txt"
done
if [[ -d "$OUT/PhemeReasoning.xcframework" ]]; then
    rm -rf "$OUT/PhemeReasoning.xcframework"
fi
xcodebuild -create-xcframework \
    -framework "$OUT/aarch64-apple-ios/PhemeReasoning.framework" \
    -framework "$OUT/aarch64-apple-ios-sim/PhemeReasoning.framework" \
    -output "$OUT/PhemeReasoning.xcframework"
echo "Packaged $OUT/PhemeReasoning.xcframework"
