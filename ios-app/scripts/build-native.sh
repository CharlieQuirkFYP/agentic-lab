#!/usr/bin/env bash
set -euo pipefail
APP=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
bash "$APP/../pheme-va/scripts/build-ios.sh" whisper
bash "$APP/../pheme-va/scripts/build-ios-reply.sh"
node "$APP/scripts/prepare-native.mjs"
