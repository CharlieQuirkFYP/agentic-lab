#!/usr/bin/env bash
set -euo pipefail
APP=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
# Comma-separated manifest IDs; defaults retain the current prepared selection.
# Example: --asr-models whisper-large-v3-turbo,whisper-tiny.en
#          --reasoning-models qwen3-4b-instruct-2507-q4-k-m,qwen2.5-1.5b-instruct-q4-k-m
# Environment alternatives: PHEME_IOS_ASR_MODELS / PHEME_IOS_REASONING_MODELS.
node "$APP/scripts/model-bundle.mjs" --download "$@"
node "$APP/scripts/prepare-native.mjs" --bundle-models "$@"
