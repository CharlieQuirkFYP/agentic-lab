#!/usr/bin/env bash
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
VARIANT=${1:-whisper}
case "$VARIANT" in whisper|disabled|test) ;; *) echo "Expected whisper, disabled, or test" >&2; exit 2 ;; esac
OUT="$ROOT/target/ios/$VARIANT"
export PHEME_XCFRAMEWORK="$OUT/PhemeVA.xcframework"
export PHEME_SWIFT_FLAGS=""
export PHEME_VA_TEST_MODEL="" PHEME_VA_TEST_AUDIO=""
if [[ "$VARIANT" == whisper ]]; then PHEME_SWIFT_FLAGS=PHEME_WHISPER; fi
if [[ "$VARIANT" == test ]]; then PHEME_SWIFT_FLAGS=PHEME_TEST_SUPPORT; fi
if [[ "${PHEME_IOS_MODEL_TEST:-0}" == 1 ]]; then
    export PHEME_VA_TEST_MODEL="$ROOT/target/ios/assets/ggml-tiny.en.bin"
    export PHEME_VA_TEST_AUDIO="$ROOT/target/ios/assets/jfk.wav"
    [[ -f "$PHEME_VA_TEST_MODEL" && -f "$PHEME_VA_TEST_AUDIO" ]]
fi
[[ -d "$PHEME_XCFRAMEWORK" ]] || { echo "Run build-ios.sh $VARIANT first." >&2; exit 1; }
mkdir -p "$OUT/project"
xcodegen generate --spec "$ROOT/hosts/ios/project.yml" --project "$OUT/project"

# Select an installed available iPhone runtime rather than relying on a runner's default device.
SIMULATOR=$(xcrun simctl list devices available -j | ruby -rjson -e '
devices = JSON.parse(STDIN.read).fetch("devices").values.flatten
device = devices.find { |d| d["name"].start_with?("iPhone") && d["isAvailable"] }
abort "No available iPhone simulator" unless device
puts device.fetch("udid")')
xcrun simctl boot "$SIMULATOR" || xcrun simctl list devices booted | grep -F "$SIMULATOR"
xcrun simctl bootstatus "$SIMULATOR" -b
xcodebuild test -project "$OUT/project/PhemePhone.xcodeproj" -scheme PhemePhone \
    -destination "platform=iOS Simulator,id=$SIMULATOR" \
    -derivedDataPath "$OUT/DerivedData" -resultBundlePath "$OUT/Tests-$(date +%s).xcresult" \
    CODE_SIGNING_ALLOWED=NO
# Also link the app for the device slice. Compilation/linking require no signing identity.
xcodebuild build -project "$OUT/project/PhemePhone.xcodeproj" -scheme PhemePhone \
    -destination 'generic/platform=iOS' -derivedDataPath "$OUT/DeviceDerivedData" \
    CODE_SIGNING_ALLOWED=NO
