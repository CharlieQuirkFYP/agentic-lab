#!/usr/bin/env bash
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
[[ $(uname -s) == Darwin ]] || { echo "Swift native tests require macOS." >&2; exit 1; }
TASK_OUT=$(mktemp -d "${TMPDIR:-/tmp}/pheme-native-tests.XXXXXX")
trap 'rm -rf "$TASK_OUT"' EXIT
swiftc -parse-as-library -swift-version 5 -strict-concurrency=complete -warnings-as-errors \
  -module-cache-path "$TASK_OUT/module-cache" \
  "$ROOT/../pheme-va/hosts/ios/Sources/BridgeTypes.swift" \
  "$ROOT/modules/pheme-native/ios/PhemeRuntime.swift" \
  "$ROOT/modules/pheme-native/ios/PhemeAudio.swift" \
  "$ROOT/tests/NativeRuntimeTests.swift" -framework AVFoundation -o "$TASK_OUT/runtime-tests"
"$TASK_OUT/runtime-tests"
