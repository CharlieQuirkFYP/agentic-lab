#!/usr/bin/env bash
set -euo pipefail
APP=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
export PHEME_VA_MODEL_DIR="$APP/../pheme-va/models"
bash "$APP/../pheme-va/scripts/download-model.sh" whisper
bash "$APP/../pheme-va/scripts/download-model.sh" qwen2.5-1.5b-instruct-q4-k-m
node "$APP/scripts/prepare-native.mjs" --bundle-models
