#!/usr/bin/env bash
# Real XcodeGen, no models/compiler/simulator; generated files stay in a temp tree.
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT
WORK="$TEST_DIR/workspace with spaces"
mkdir -p "$WORK/hosts" "$WORK/scripts"
cp -R "$ROOT/hosts/ios" "$WORK/hosts/"
cp "$ROOT/scripts/test-ios.sh" "$ROOT/scripts/check-ios-project.rb" "$WORK/scripts/"

expect_failure() {
    local message=$1
    shift
    if "$@" > "$TEST_DIR/failure.log" 2>&1; then
        echo "FAIL: expected rejection: $message" >&2
        exit 1
    fi
    if ! grep -Fq "$message" "$TEST_DIR/failure.log"; then
        cat "$TEST_DIR/failure.log" >&2
        exit 1
    fi
}

assert_assets() {
    ruby - "$WORK/target/ios/$1/project/PhemePhone.xcodeproj/project.pbxproj" "$2" <<'RUBY'
require "json"
path, expected = ARGV
json = IO.popen(["plutil", "-convert", "json", "-o", "-", path], &:read)
abort "Could not parse generated project" unless $?.success?
objects = JSON.parse(json).fetch("objects")
assets = objects.select do |_, obj|
  obj["isa"] == "PBXFileReference" && File.basename(obj.fetch("path", "")) == "assets"
end.keys
abort "Unexpected number of assets references" unless assets.length == Integer(expected)
%w[PhemePhone BridgeTests].each do |name|
  target = objects.values.find { |obj| obj["isa"] == "PBXNativeTarget" && obj["name"] == name }
  copied = target.fetch("buildPhases").flat_map do |id|
    phase = objects.fetch(id)
    phase["isa"] == "PBXResourcesBuildPhase" ? phase.fetch("files", []) : []
  end.count { |id| assets.include?(objects.fetch(id)["fileRef"]) }
  wanted = name == "BridgeTests" ? Integer(expected) : 0
  abort "Unexpected assets copy count in #{name}: #{copied}" unless copied == wanted
end
RUBY
}

# A clean checkout has no model assets. None of the ordinary variants may copy them.
for variant in whisper disabled test; do
    mkdir -p "$WORK/target/ios/$variant/PhemeVA.xcframework"
    PHEME_IOS_MODEL_TEST=0 bash "$WORK/scripts/test-ios.sh" "$variant" --generate-only
    assert_assets "$variant" 0
done
[[ ! -e "$WORK/target/ios/assets" ]]

# Explicit model tests require both fixtures; missing inputs must fail before generation.
expect_failure "Missing iOS model-test fixture:" env PHEME_IOS_MODEL_TEST=1 \
    bash "$WORK/scripts/test-ios.sh" whisper --generate-only
mkdir -p "$WORK/target/ios/assets"
# Tiny placeholders only exercise project generation, never inference.
touch "$WORK/target/ios/assets/ggml-tiny.en.bin"
expect_failure "jfk.wav. Run bash scripts/ios-model-fixture.sh first." env PHEME_IOS_MODEL_TEST=1 \
    bash "$WORK/scripts/test-ios.sh" whisper --generate-only
rm "$WORK/target/ios/assets/ggml-tiny.en.bin"
touch "$WORK/target/ios/assets/jfk.wav"
expect_failure "ggml-tiny.en.bin. Run bash scripts/ios-model-fixture.sh first." env PHEME_IOS_MODEL_TEST=1 \
    bash "$WORK/scripts/test-ios.sh" whisper --generate-only
touch "$WORK/target/ios/assets/ggml-tiny.en.bin"
for variant in disabled test; do
    expect_failure "Model tests require the whisper variant." env PHEME_IOS_MODEL_TEST=1 \
        bash "$WORK/scripts/test-ios.sh" "$variant" --generate-only
done
PHEME_IOS_MODEL_TEST=1 bash "$WORK/scripts/test-ios.sh" whisper --generate-only
assert_assets whisper 1

PROJECT="$WORK/target/ios/whisper/project/PhemePhone.xcodeproj"
FRAMEWORK="$WORK/target/ios/whisper/PhemeVA.xcframework"
# The preflight must catch a referenced folder that disappears before Xcode runs.
mv "$WORK/target/ios/assets" "$WORK/target/ios/saved-assets"
expect_failure "Missing test assets directory:" ruby "$WORK/scripts/check-ios-project.rb" "$PROJECT" "$FRAMEWORK"
mv "$WORK/target/ios/saved-assets" "$WORK/target/ios/assets"

# Reintroduce the reported relative path and require the preflight to reject it.
ruby - "$PROJECT/project.pbxproj" <<'RUBY'
path = ARGV.fetch(0)
text = File.read(path)
before = "path = ../PhemeVA.xcframework;"
abort "Expected framework path was not generated" unless text.include?(before)
File.write(path, text.sub(before, "path = ../../target/ios/whisper/PhemeVA.xcframework;"))
RUBY
expect_failure "Incorrect XCFramework reference" ruby "$WORK/scripts/check-ios-project.rb" "$PROJECT" "$FRAMEWORK"

# Downloaded fixtures and a previously enabled model test must not affect ordinary runs.
for variant in whisper disabled test; do
    PHEME_IOS_MODEL_TEST=0 bash "$WORK/scripts/test-ios.sh" "$variant" --generate-only
    assert_assets "$variant" 0
done
echo "PASS: clean variants, opt-in assets, missing fixtures, paths with spaces and invalid-reference rejection"
