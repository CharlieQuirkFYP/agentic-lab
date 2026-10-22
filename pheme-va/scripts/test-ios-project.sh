#!/usr/bin/env bash
# Real XcodeGen, no models/compiler/simulator; generated files stay in a temp tree.
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT
WORK="$TEST_DIR/workspace with spaces"
mkdir -p "$WORK/hosts" "$WORK/scripts" "$WORK/target/ios/assets"
cp -R "$ROOT/hosts/ios" "$WORK/hosts/"
cp "$ROOT/scripts/test-ios.sh" "$ROOT/scripts/check-ios-project.rb" "$WORK/scripts/"
# Tiny placeholders only exercise project generation, never inference.
touch "$WORK/target/ios/assets/ggml-tiny.en.bin" "$WORK/target/ios/assets/jfk.wav"
for variant in whisper disabled test; do
    mkdir -p "$WORK/target/ios/$variant/PhemeVA.xcframework"
    PHEME_IOS_MODEL_TEST=0 bash "$WORK/scripts/test-ios.sh" "$variant" --generate-only
done
PHEME_IOS_MODEL_TEST=1 bash "$WORK/scripts/test-ios.sh" whisper --generate-only

# Reintroduce the reported relative path and require the preflight to reject it.
PROJECT="$WORK/target/ios/whisper/project/PhemePhone.xcodeproj"
ruby - "$PROJECT/project.pbxproj" <<'RUBY'
path = ARGV.fetch(0)
text = File.read(path)
before = "path = ../PhemeVA.xcframework;"
abort "Expected framework path was not generated" unless text.include?(before)
File.write(path, text.sub(before, "path = ../../target/ios/whisper/PhemeVA.xcframework;"))
RUBY
if ruby "$WORK/scripts/check-ios-project.rb" "$PROJECT" \
    "$WORK/target/ios/whisper/PhemeVA.xcframework" > "$TEST_DIR/failure.log" 2>&1; then
    echo "FAIL: duplicated target/ios path passed preflight" >&2
    exit 1
fi
grep -q "Incorrect XCFramework reference" "$TEST_DIR/failure.log"
echo "PASS: all variants, optional assets, paths with spaces and duplicated-path rejection"
