#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
MODEL_DIR="$SCRIPT_DIR/../models"
MODEL_PATH="$MODEL_DIR/ggml-large-v3-turbo.bin"
PART_PATH="$MODEL_PATH.part"
MODEL_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin"
EXPECTED_SHA256="1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69"

checksum() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        printf 'download-model: sha256sum or shasum is required\n' >&2
        exit 1
    fi
}

verify() {
    local path=$1 actual
    actual=$(checksum "$path")
    if [[ "$actual" != "$EXPECTED_SHA256" ]]; then
        printf 'download-model: checksum mismatch for %s\nexpected: %s\nactual:   %s\n' \
            "$path" "$EXPECTED_SHA256" "$actual" >&2
        return 1
    fi
}

mkdir -p "$MODEL_DIR"

if [[ -f "$MODEL_PATH" ]]; then
    verify "$MODEL_PATH"
    printf 'Whisper model already present and verified: %s\n' "$MODEL_PATH"
    exit 0
fi

printf 'Downloading Whisper model to %s\n' "$MODEL_PATH"
curl --fail --location --retry 3 --retry-all-errors --continue-at - \
    --output "$PART_PATH" "$MODEL_URL"
verify "$PART_PATH"
mv "$PART_PATH" "$MODEL_PATH"
printf 'Downloaded and verified: %s\n' "$MODEL_PATH"
