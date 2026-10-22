#!/usr/bin/env bash
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"
# Avoid running host CPU feature probes; use the same conservative baseline as iOS.
export GGML_NATIVE=OFF GGML_CPU_ARM_ARCH=armv8-a
for VARIANT in disabled whisper test; do
    OUT="$ROOT/target/ffi-swift/$VARIANT"
    mkdir -p "$OUT/include"
    cp include/pheme_va.h include/module.modulemap "$OUT/include/"
    FLAGS=(-D PHEME_DESKTOP_SMOKE)
    CARGO_FLAGS=(--no-default-features)
    export RUSTFLAGS=""
    unset CARGO_ENCODED_RUSTFLAGS
    if [[ "$VARIANT" == whisper ]]; then
        CARGO_FLAGS+=(--features whisper)
        FLAGS+=(-D PHEME_WHISPER)
    elif [[ "$VARIANT" == test ]]; then
        export RUSTFLAGS="--cfg pheme_ffi_test"
        FLAGS+=(-D PHEME_TEST_SUPPORT)
        printf '\nPhemeVaHandle *pheme_va_test_new(void);\nPhemeVaHandle *pheme_va_test_create(void);\n' >> "$OUT/include/pheme_va.h"
    fi
    cargo build --locked -p ffi --lib --target-dir "$OUT/cargo" "${CARGO_FLAGS[@]}"
    swiftc -parse-as-library -I "$OUT/include" -module-cache-path "$OUT/module-cache" \
        "${FLAGS[@]}" hosts/ios/Sources/NativeBridge.swift hosts/ios/Tests/FFISmoke.swift \
        "$OUT/cargo/debug/libffi.a" -framework Accelerate -framework Security \
        -framework SystemConfiguration -lc++ -lresolv -o "$OUT/smoke"
    "$OUT/smoke"
done
