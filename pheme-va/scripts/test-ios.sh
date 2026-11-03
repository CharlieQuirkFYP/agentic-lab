#!/usr/bin/env bash
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
VARIANT=${1:-whisper}
case "$VARIANT" in whisper|disabled|test) ;; *) echo "Expected whisper, disabled, or test" >&2; exit 2 ;; esac
[[ $# -le 2 && ( -z "${2:-}" || "${2:-}" == --generate-only ) ]] || {
    echo "Usage: bash scripts/test-ios.sh [whisper|disabled|test] [--generate-only]" >&2; exit 2;
}
OUT="$ROOT/target/ios/$VARIANT"
SPEC="$ROOT/hosts/ios/project.yml"
export PHEME_XCFRAMEWORK="$OUT/PhemeVA.xcframework"
export PHEME_SWIFT_FLAGS=""
export PHEME_VA_TEST_MODEL="" PHEME_VA_TEST_AUDIO=""
export PHEME_VA_TEST_REPLY_MODEL="${PHEME_VA_TEST_REPLY_MODEL:-}"
# Project generation must not erase a developer's selected signing identity.
PREVIOUS_PROJECT="$OUT/project/PhemePhone.xcodeproj/project.pbxproj"
SAVED_SIGNING=()
if [[ -f "$PREVIOUS_PROJECT" ]]; then
    while IFS= read -r setting; do SAVED_SIGNING+=("$setting"); done < <(
        ruby -rjson - "$PREVIOUS_PROJECT" <<'RUBY'
json = IO.popen(["plutil", "-convert", "json", "-o", "-", ARGV.fetch(0)], &:read)
abort "Cannot read previous signing settings" unless $?.success?
objects = JSON.parse(json).fetch("objects")
target = objects.values.find { |obj| obj["isa"] == "PBXNativeTarget" && obj["name"] == "PhemePhone" }
configs = objects.fetch(target.fetch("buildConfigurationList")).fetch("buildConfigurations")
settings = configs.map { |id| objects.fetch(id) }.find { |obj| obj["name"] == "Debug" }.fetch("buildSettings")
puts settings.fetch("DEVELOPMENT_TEAM", "")
puts settings.fetch("PRODUCT_BUNDLE_IDENTIFIER", "org.agenticlab.PhemePhone")
RUBY
    )
fi
export PHEME_PHONE_TEAM="${PHEME_PHONE_TEAM:-${SAVED_SIGNING[0]:-}}"
export PHEME_PHONE_BUNDLE_ID="${PHEME_PHONE_BUNDLE_ID:-${SAVED_SIGNING[1]:-org.agenticlab.PhemePhone}}"
CHECK_FRAMEWORKS=("$PHEME_XCFRAMEWORK")
if [[ "$VARIANT" == whisper ]]; then PHEME_SWIFT_FLAGS=PHEME_WHISPER; fi
if [[ "$VARIANT" == test ]]; then PHEME_SWIFT_FLAGS=PHEME_TEST_SUPPORT; fi
if [[ "${PHEME_IOS_APP_MODELS:-0}" == 1 ]]; then
    [[ "$VARIANT" == whisper ]] || { echo "Bundled app requires the whisper variant." >&2; exit 2; }
    [[ "${PHEME_IOS_MODEL_TEST:-0}" != 1 ]] || { echo "Choose bundled app or fixture tests, not both." >&2; exit 2; }
    export PHEME_REASONING_XCFRAMEWORK="$ROOT/target/ios/reply/PhemeReasoning.xcframework"
    [[ -d "$PHEME_REASONING_XCFRAMEWORK" ]] || { echo "Run build-ios-reply.sh first." >&2; exit 1; }
    for fixture in \
        "$ROOT/target/ios/app-models/PhemeModels/ASR/ggml-large-v3-turbo.bin" \
        "$ROOT/target/ios/app-models/PhemeModels/Reasoning/qwen2.5-1.5b-instruct-q4_k_m.gguf" \
        "$ROOT/target/ios/app-models/PhemeModels/incident-reporting.txt"; do
        [[ -f "$fixture" ]] || { echo "Missing bundled model asset: $fixture. Run build-phone.sh first." >&2; exit 1; }
    done
    SPEC="$ROOT/hosts/ios/project-local-models.yml"
    PHEME_SWIFT_FLAGS="$PHEME_SWIFT_FLAGS PHEME_REASONING"
    CHECK_FRAMEWORKS+=("$PHEME_REASONING_XCFRAMEWORK")
fi
if [[ "${PHEME_IOS_MODEL_TEST:-0}" == 1 ]]; then
    [[ "$VARIANT" == whisper ]] || { echo "Model tests require the whisper variant." >&2; exit 2; }
    export PHEME_VA_TEST_MODEL="$ROOT/target/ios/assets/ggml-tiny.en.bin"
    export PHEME_VA_TEST_AUDIO="$ROOT/target/ios/assets/jfk.wav"
    for fixture in "$PHEME_VA_TEST_MODEL" "$PHEME_VA_TEST_AUDIO"; do
        [[ -f "$fixture" ]] || {
            echo "Missing iOS model-test fixture: $fixture. Run bash scripts/ios-model-fixture.sh first." >&2
            exit 1
        }
    done
    SPEC="$ROOT/hosts/ios/project-model-test.yml"
fi
[[ -d "$PHEME_XCFRAMEWORK" ]] || { echo "Run build-ios.sh $VARIANT first." >&2; exit 1; }
mkdir -p "$OUT/project"
xcodegen generate --spec "$SPEC" \
    --project-root "$OUT/project" --project "$OUT/project"
ruby "$ROOT/scripts/check-ios-project.rb" "$OUT/project/PhemePhone.xcodeproj" "${CHECK_FRAMEWORKS[@]}"
if [[ "${2:-}" == --generate-only ]]; then exit 0; fi

# Select an installed available iPhone runtime rather than relying on a runner's default device.
SIMULATOR=$(xcrun simctl list devices available -j | ruby -rjson -e '
devices = JSON.parse(STDIN.read).fetch("devices").values.flatten
device = devices.find { |d| d["name"].start_with?("iPhone") && d["isAvailable"] }
abort "No available iPhone simulator" unless device
puts device.fetch("udid")')
xcrun simctl boot "$SIMULATOR" || xcrun simctl list devices booted | grep -F "$SIMULATOR"
xcrun simctl bootstatus "$SIMULATOR" -b
xcodebuild test -project "$OUT/project/PhemePhone.xcodeproj" -scheme PhemePhone \
    -destination "platform=iOS Simulator,id=$SIMULATOR,arch=arm64" \
    -derivedDataPath "$OUT/DerivedData" -resultBundlePath "$OUT/Tests-$(date +%s).xcresult" \
    CODE_SIGNING_ALLOWED=NO
# Also link the app for the device slice. Compilation/linking require no signing identity.
xcodebuild build -project "$OUT/project/PhemePhone.xcodeproj" -scheme PhemePhone \
    -destination 'generic/platform=iOS' -derivedDataPath "$OUT/DeviceDerivedData" \
    CODE_SIGNING_ALLOWED=NO
