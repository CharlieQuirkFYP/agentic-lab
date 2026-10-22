#!/usr/bin/env bash
# Offline orchestration checks; native compilation/linking still belongs to iOS CI.
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT
export FAKE_ROOT="$TEST_DIR/workspace with spaces"
export FAKE_SYSROOT="$TEST_DIR/rust toolchain"
export FAKE_LOG="$TEST_DIR/commands.log"
mkdir -p "$FAKE_ROOT/scripts" "$FAKE_ROOT/include" "$TEST_DIR/bin" \
    "$FAKE_SYSROOT/lib/rustlib/aarch64-apple-darwin/bin"
cp "$ROOT/scripts/build-ios.sh" "$FAKE_ROOT/scripts/"
cp "$ROOT/include/pheme_va.h" "$ROOT/include/module.modulemap" "$FAKE_ROOT/include/"
cp "$ROOT/Cargo.lock" "$FAKE_ROOT/"

cat > "$TEST_DIR/bin/fake-tool" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
tool=${0##*/}
printf '%s %s\n' "$tool" "$*" >> "$FAKE_LOG"
case "$tool" in
    uname) echo arm64 ;;
    rustup) ;;
    rustc)
        case "$*" in
            "--print sysroot") echo "$FAKE_SYSROOT" ;;
            *) printf 'rustc 1.99.0\nhost: aarch64-apple-darwin\n' ;;
        esac
        ;;
    cargo)
        case "$1" in
            --version) echo 'cargo 1.99.0' ;;
            fetch)
                [[ "$*" == "fetch --locked --target aarch64-apple-ios --target aarch64-apple-ios-sim" ]]
                exit "${FAKE_FETCH_STATUS:-0}"
                ;;
            build)
                [[ "$2" == --frozen ]]
                [[ "$RUSTUP_TOOLCHAIN" == 1.99.0 ]]
                [[ -z "${CARGO_ENCODED_RUSTFLAGS+x}" ]]
                printf 'rustflags=%s\n' "$RUSTFLAGS" >> "$FAKE_LOG"
                target=${!#}
                mkdir -p "$CARGO_TARGET_DIR/$target/release"
                printf 'fake archive\n' > "$CARGO_TARGET_DIR/$target/release/libffi.a"
                ;;
            *) exit 90 ;;
        esac
        ;;
    llvm-nm)
        if [[ "$1" == --version ]]; then echo 'LLVM version 23'; exit 0; fi
        [[ "$1" == --extern-only && "$2" == --defined-only ]]
        [[ -f "$3" ]]
        if [[ "${FAKE_NM_STATUS:-0}" != 0 ]]; then exit "$FAKE_NM_STATUS"; fi
        if [[ "${FAKE_MISSING_ABI:-0}" != 1 ]]; then echo '00000000 T _pheme_va_abi_version'; fi
        if [[ "$3" == */test/* || "${FAKE_LEAK:-0}" == 1 ]]; then
            echo '00000000 T _pheme_va_test_create'
        fi
        if [[ "$3" == */whisper/* || "${FAKE_LEGACY:-0}" == 1 ]]; then
            echo '00000000 T _pheme_va_whisper_new'
        fi
        ;;
    xcrun)
        # Using Apple's nm must fail this suite, even when output looks valid.
        [[ "$1" != nm ]] || exit 91
        echo '/fake/Apple SDK'
        ;;
    xcodebuild)
        if [[ "$1" == -create-xcframework ]]; then mkdir -p "${!#}"; fi
        echo 'Xcode fake'
        ;;
    git|xcodegen|cmake) echo 'fake version' ;;
    *) exit 92 ;;
esac
EOF
chmod +x "$TEST_DIR/bin/fake-tool"
for tool in uname rustup rustc cargo xcrun xcodebuild git xcodegen cmake; do
    ln -s fake-tool "$TEST_DIR/bin/$tool"
done
ln -s "$TEST_DIR/bin/fake-tool" "$FAKE_SYSROOT/lib/rustlib/aarch64-apple-darwin/bin/llvm-nm"
export PATH="$TEST_DIR/bin:$PATH"
# The build must discard unrelated caller/test flags.
export RUSTFLAGS="--cfg unrelated_test" CARGO_ENCODED_RUSTFLAGS="--cfg=unrelated_test"
unset PHEME_IOS_RUST_TOOLCHAIN

fail() { echo "FAIL: $*" >&2; exit 1; }
reset_case() {
    rm -rf "$FAKE_ROOT/target"
    : > "$FAKE_LOG"
    unset FAKE_FETCH_STATUS FAKE_NM_STATUS FAKE_MISSING_ABI FAKE_LEAK FAKE_LEGACY
}
run_build() {
    local variant=$1 expected=$2 status=0
    bash "$FAKE_ROOT/scripts/build-ios.sh" "$variant" > "$TEST_DIR/output.log" 2>&1 || status=$?
    if [[ "$status" != "$expected" ]]; then
        cat "$TEST_DIR/output.log" >&2
        fail "$variant returned $status, expected $expected"
    fi
}
assert_not_packaged() {
    [[ ! -d "$FAKE_ROOT/target/ios/$1/PhemeVA.xcframework" ]] || fail 'packaged after failure'
    ! grep -q '^xcodebuild -create-xcframework' "$FAKE_LOG" || fail 'packaging attempted after failure'
}

for variant in whisper disabled test; do
    reset_case
    run_build "$variant" 0
    out="$FAKE_ROOT/target/ios/$variant"
    [[ -d "$out/PhemeVA.xcframework" ]] || fail 'missing package'
    [[ $(grep -c '^cargo fetch ' "$FAKE_LOG") == 1 ]] || fail 'expected one fetch'
    [[ $(grep -c '^cargo build --frozen ' "$FAKE_LOG") == 2 ]] || fail 'expected two frozen builds'
    [[ $(grep -c '^llvm-nm --extern-only --defined-only ' "$FAKE_LOG") == 2 ]] || fail 'both slices must be inspected'
    grep -q '^rustup component add llvm-tools-preview$' "$FAKE_LOG"
    grep -q 'LLVM version 23' "$out/build-metadata.txt"
    grep -q 'aarch64-apple-ios/release/libffi.a' "$out/build-metadata.txt"
    grep -q 'aarch64-apple-ios-sim/release/libffi.a' "$out/build-metadata.txt"
    if [[ "$variant" == test ]]; then
        [[ $(grep -c '^rustflags=--cfg pheme_ffi_test$' "$FAKE_LOG") == 2 ]]
        grep -q 'pheme_va_test_create' "$out/headers/pheme_va.h"
    else
        [[ $(grep -c '^rustflags=$' "$FAKE_LOG") == 2 ]]
        ! grep -q 'pheme_va_test_create' "$out/headers/pheme_va.h"
    fi
    if [[ "$variant" == whisper ]]; then
        [[ $(grep -c '^cargo build .*--features whisper ' "$FAKE_LOG") == 2 ]]
    else
        ! grep -q '^cargo build .*--features' "$FAKE_LOG"
    fi
done

reset_case
export FAKE_FETCH_STATUS=101
run_build whisper 101
assert_not_packaged whisper
! grep -q '^cargo build ' "$FAKE_LOG" || fail 'build ran after failed fetch'
grep -q 'rustc 1.99.0' "$FAKE_ROOT/target/ios/whisper/build-metadata.txt"

reset_case
export FAKE_NM_STATUS=42
run_build disabled 42
assert_not_packaged disabled

reset_case
export FAKE_LEAK=1
run_build whisper 1
assert_not_packaged whisper
grep -q 'Test hooks leaked' "$TEST_DIR/output.log"

reset_case
export FAKE_LEGACY=1
run_build disabled 1
assert_not_packaged disabled
grep -q 'Legacy Whisper symbols' "$TEST_DIR/output.log"

reset_case
export FAKE_MISSING_ABI=1
run_build disabled 1
assert_not_packaged disabled
grep -q 'Stable ABI entry point missing' "$TEST_DIR/output.log"

reset_case
run_build invalid 2
[[ ! -s "$FAKE_LOG" ]] || fail 'invalid variant invoked build tools'
echo 'PASS: iOS packaging variants, toolchain symbols, frozen builds, diagnostics and failure gates'
