#!/usr/bin/env bash
# Explicit opt-in: a development app with bundled ASR and reasoning weights.
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
[[ $# -le 1 && ( $# == 0 || "$1" == --prepare-only ) ]] || {
    echo "Usage: bash scripts/build-phone.sh [--prepare-only]" >&2; exit 2;
}
cd "$ROOT"
export PHEME_VA_MODEL_DIR="$ROOT/models"
bash scripts/download-model.sh whisper
bash scripts/download-model.sh qwen2.5-1.5b-instruct-q4-k-m
OUT="$ROOT/target/ios/app-models/PhemeModels"
mkdir -p "$OUT/ASR" "$OUT/Reasoning"
# Read weights in the app bundle, rather than copying them again on the phone.
cp "$ROOT/models/transcript/whisper/ggml-large-v3-turbo.bin" "$OUT/ASR/"
cp "$ROOT/models/reply/qwen2.5-1.5b-instruct-q4_k_m.gguf" "$OUT/Reasoning/"
cp "$ROOT/roles/incident-reporting.txt" "$OUT/"
cat > "$OUT/MODEL-NOTICES.txt" <<'NOTICES'
Pheme development model bundle

ASR: Whisper large-v3-turbo GGML, MIT
Source: ggerganov/whisper.cpp
Revision: 5359861c739e955e79d9a303bcbc70fb988958b1
SHA-256: 1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69

Reasoning: Qwen2.5-1.5B-Instruct GGUF Q4_K_M, Apache-2.0
Source: Qwen/Qwen2.5-1.5B-Instruct-GGUF
Revision: 91cad51170dc346986eccefdc2dd33a9da36ead9
SHA-256: 6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e

These are development baselines, not validated phone selections.
Review upstream model licences and include full required licence texts before
distributing this app. This script does not perform App Store distribution.
NOTICES
if [[ "${1:-}" == --prepare-only ]]; then
    echo "Prepared bundled models: $OUT"
    exit 0
fi
bash scripts/build-ios.sh whisper
bash scripts/build-ios-reply.sh
PHEME_IOS_APP_MODELS=1 bash scripts/test-ios.sh whisper --generate-only
echo "Open $ROOT/target/ios/whisper/project/PhemePhone.xcodeproj"
