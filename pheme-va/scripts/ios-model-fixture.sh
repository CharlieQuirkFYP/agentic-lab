#!/usr/bin/env bash
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
OUT="$ROOT/target/ios/assets"
mkdir -p "$OUT"
download() {
    local name=$1 url=$2 expected=$3
    curl --fail --location --retry 3 --max-time 300 --output "$OUT/$name.part" "$url"
    local actual
    actual=$(shasum -a 256 "$OUT/$name.part" | awk '{print $1}')
    [[ "$actual" == "$expected" ]] || { echo "Checksum mismatch: $name" >&2; exit 1; }
    mv "$OUT/$name.part" "$OUT/$name"
}
download ggml-tiny.en.bin \
    https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-tiny.en.bin \
    921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f
# Upstream whisper.cpp test audio, JFK's public-domain US government speech.
# Content is pinned by SHA-256 even if the upstream release tag moves.
download jfk.wav \
    https://raw.githubusercontent.com/ggml-org/whisper.cpp/v1.7.6/samples/jfk.wav \
    59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e
