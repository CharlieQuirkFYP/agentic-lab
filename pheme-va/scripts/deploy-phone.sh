#!/usr/bin/env bash
# Builds the bundled native app; the desktop API is not part of this package.
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
usage() {
    echo "Usage: bash scripts/deploy-phone.sh [simulator|device|archive] [--use-prepared] [--build-only]" >&2
}
MODE=${1:-simulator}
if [[ $# -gt 0 ]]; then shift; fi
case "$MODE" in simulator|device|archive) ;; *) usage; exit 2 ;; esac
PREPARED=0
BUILD_ONLY=0
for option in "$@"; do
    case "$option" in
        --use-prepared) PREPARED=1 ;;
        --build-only) BUILD_ONLY=1 ;;
        *) usage; exit 2 ;;
    esac
done
BUNDLE_ID=${PHEME_IOS_BUNDLE_ID:-org.agenticlab.PhemePhone}
TEAM=${PHEME_IOS_DEVELOPMENT_TEAM:-}
[[ "$BUNDLE_ID" =~ ^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$ ]] || {
    echo "PHEME_IOS_BUNDLE_ID must be a reverse-DNS bundle identifier." >&2; exit 2;
}
if [[ "$MODE" != simulator && -z "$TEAM" ]]; then
    echo "Set PHEME_IOS_DEVELOPMENT_TEAM to your Apple development team ID." >&2
    exit 2
fi
if [[ "$MODE" == device && -z "${PHEME_IOS_DEVICE_ID:-}" ]]; then
    echo "Set PHEME_IOS_DEVICE_ID from: xcrun devicectl list devices" >&2
    exit 2
fi
OUT="$ROOT/target/ios/deploy/$MODE"
mkdir -p "$OUT"
export PHEME_IOS_MODEL_TEST=0 PHEME_IOS_APP_MODELS=1
if [[ "$PREPARED" == 0 ]]; then
    bash "$ROOT/scripts/build-phone.sh"
fi
# Keep deployment generation separate from ongoing ABI/simulator test projects.
PROJECT_ROOT="$ROOT/target/ios/phone-$MODE/project"
export PHEME_XCFRAMEWORK="$ROOT/target/ios/whisper/PhemeVA.xcframework"
export PHEME_REASONING_XCFRAMEWORK="$ROOT/target/ios/reply/PhemeReasoning.xcframework"
export PHEME_SWIFT_FLAGS="PHEME_WHISPER PHEME_REASONING"
export PHEME_VA_TEST_MODEL="" PHEME_VA_TEST_AUDIO="" PHEME_VA_TEST_REPLY_MODEL=""
for framework in "$PHEME_XCFRAMEWORK" "$PHEME_REASONING_XCFRAMEWORK"; do
    [[ -d "$framework" ]] || { echo "Missing native framework: $framework. Run build-phone.sh first." >&2; exit 1; }
done
mkdir -p "$PROJECT_ROOT"
xcodegen generate --spec "$ROOT/hosts/ios/project-local-models.yml" \
    --project-root "$PROJECT_ROOT" --project "$PROJECT_ROOT"
PROJECT="$PROJECT_ROOT/PhemePhone.xcodeproj"
[[ -d "$PROJECT" ]] || { echo "Missing bundled phone project: $PROJECT" >&2; exit 1; }
ruby "$ROOT/scripts/check-ios-project.rb" "$PROJECT" "$PHEME_XCFRAMEWORK" "$PHEME_REASONING_XCFRAMEWORK"
COMMON=(-project "$PROJECT" -scheme PhemePhone -configuration Release
    -derivedDataPath "$OUT/DerivedData" "PRODUCT_BUNDLE_IDENTIFIER=$BUNDLE_ID"
    ARCHS=arm64 ONLY_ACTIVE_ARCH=YES)
{
    printf 'mode=%s\nbundle_identifier=%s\nsigning_team=%s\n' "$MODE" "$BUNDLE_ID" "$TEAM"
    git -C "$ROOT" rev-parse HEAD
    xcodebuild -version
} > "$OUT/deployment-metadata.txt"
if [[ "$MODE" == simulator ]]; then
    SIMULATOR=${PHEME_IOS_SIMULATOR_ID:-}
    if [[ -z "$SIMULATOR" ]]; then
        SIMULATOR=$(xcrun simctl list devices available -j | ruby -rjson -e '
devices = JSON.parse(STDIN.read).fetch("devices").values.flatten
phones = devices.select { |d| d["name"].start_with?("iPhone") && d["isAvailable"] }
device = phones.find { |d| d["state"] == "Booted" } || phones.first
abort "No available iPhone simulator. Install an iOS runtime in Xcode." unless device
puts device.fetch("udid")')
    fi
    xcodebuild build "${COMMON[@]}" -destination "platform=iOS Simulator,id=$SIMULATOR,arch=arm64" \
        CODE_SIGNING_ALLOWED=NO 2>&1 | tee "$OUT/build.log"
    APP="$OUT/DerivedData/Build/Products/Release-iphonesimulator/PhemePhone.app"
else
    SIGNING=("DEVELOPMENT_TEAM=$TEAM" CODE_SIGN_STYLE=Automatic -allowProvisioningUpdates)
    if [[ "$MODE" == archive ]]; then
        xcodebuild archive "${COMMON[@]}" -destination 'generic/platform=iOS' \
            -archivePath "$OUT/PhemePhone.xcarchive" "${SIGNING[@]}" 2>&1 | tee "$OUT/build.log"
        APP="$OUT/PhemePhone.xcarchive/Products/Applications/PhemePhone.app"
    else
        xcodebuild build "${COMMON[@]}" -destination "platform=iOS,id=$PHEME_IOS_DEVICE_ID" \
            "${SIGNING[@]}" -allowProvisioningDeviceRegistration 2>&1 | tee "$OUT/build.log"
        APP="$OUT/DerivedData/Build/Products/Release-iphoneos/PhemePhone.app"
    fi
fi
ruby "$ROOT/scripts/check-phone-app.rb" "$APP" "$BUNDLE_ID" | tee "$OUT/bundle-validation.txt"
if [[ "$MODE" != simulator ]]; then
    codesign --verify --deep --strict --verbose=2 "$APP"
    codesign --display --verbose=4 "$APP" 2> "$OUT/signing.txt"
fi
if [[ "$MODE" == archive ]]; then
    xcodebuild -exportArchive -archivePath "$OUT/PhemePhone.xcarchive" \
        -exportOptionsPlist "$ROOT/hosts/ios/ExportOptions.plist" \
        -exportPath "$OUT/export" -allowProvisioningUpdates 2>&1 | tee "$OUT/export.log"
    [[ -f "$OUT/export/PhemePhone.ipa" ]] || { echo "IPA export did not produce PhemePhone.ipa." >&2; exit 1; }
    unzip -tq "$OUT/export/PhemePhone.ipa" | tee "$OUT/ipa-validation.txt"
    shasum -a 256 "$OUT/export/PhemePhone.ipa" | tee "$OUT/ipa-sha256.txt"
    echo "Exported development IPA: $OUT/export/PhemePhone.ipa"
elif [[ "$BUILD_ONLY" == 0 ]]; then
    if [[ "$MODE" == simulator ]]; then
        if ! xcrun simctl list devices booted -j | ruby -rjson -e '
id = ARGV.fetch(0)
exit(JSON.parse(STDIN.read).fetch("devices").values.flatten.any? { |d| d["udid"] == id } ? 0 : 1)' "$SIMULATOR"; then
            xcrun simctl boot "$SIMULATOR"
        fi
        xcrun simctl bootstatus "$SIMULATOR" -b
        xcrun simctl install "$SIMULATOR" "$APP"
        xcrun simctl launch "$SIMULATOR" "$BUNDLE_ID"
    else
        xcrun devicectl device install app --device "$PHEME_IOS_DEVICE_ID" "$APP"
        xcrun devicectl device process launch --device "$PHEME_IOS_DEVICE_ID" "$BUNDLE_ID"
    fi
fi
echo "Validated app: $APP"
echo "Logs and deployment metadata: $OUT"
