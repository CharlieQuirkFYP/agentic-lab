#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
MODEL_DIR="$SCRIPT_DIR/../models"

WHISPER_REPO="ggerganov/whisper.cpp"
WHISPER_REVISION="5359861c739e955e79d9a303bcbc70fb988958b1"
WHISPER_URL="https://huggingface.co/$WHISPER_REPO/resolve/$WHISPER_REVISION/ggml-large-v3-turbo.bin"
WHISPER_SHA256="1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69"

LITERT_REPO="litert-community/Zipformer-medium-CR-CTC-LiteRT"
LITERT_REVISION="7732ad6c15ec43402968d5ae04acfa7a204027f5"
LITERT_BASE_URL="https://huggingface.co/$LITERT_REPO/resolve/$LITERT_REVISION"
BPE_SHA256="c53433de083c4a6ad12d034550ef22de68cec62c4f58932a7b6b8b2f1e743fa5"
TOKENS_SHA256="49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb"
ZIPFORMER_SMALL_SHA256="ff6d70c7e8cfdfcf994be625456ca6543e0dbcc76fd4fd428e2138642d0852ba"
ZIPFORMER_MEDIUM_SHA256="9515d94c04306798fecfe695eb8f79cc8ac98ed3d8eab81c28ac464b7dbddf48"
ZIPFORMER_LARGE_SHA256="183c928cd1b109ad0b94d9540dbf4c2660393428d2104494b6047af4aa4eb1da"

SEALLMS_REPO="SeaLLMs/SeaLLMs-Audio-7B"
SEALLMS_REVISION="c5c3152b373350653ec6dd981d041f8285ab26c9"
SEALLMS_BASE_URL="https://huggingface.co/$SEALLMS_REPO/resolve/$SEALLMS_REVISION"
SEALLMS_MODEL_1_SHA256="6ab818c2f728fc9030bfe149c3ed666a8770bba1cedb18a9dc81e1db64213a21"
SEALLMS_MODEL_2_SHA256="8fb2dc90d06f8392dcc9f9f0fb01cdc88e86d531babcccd04c18ce0c136295a1"
SEALLMS_MODEL_3_SHA256="472cead59645d4f981b9f8cac931e5a334ad1b0b3baf794016df203ec43b6992"
SEALLMS_MODEL_4_SHA256="e50c202a880e492c974d982d4b802d28477618dd65f2eea08849ac522ad4cee1"
SEALLMS_TOKENIZER_SHA256="fecdb47d281073055efd605d080013e3114ed0f3c5d8af201e245b199864c9c7"

usage() {
    cat <<'EOF'
Usage:
  ./scripts/download-model.sh                         Download the Whisper baseline
  ./scripts/download-model.sh whisper                 Download the Whisper baseline
  ./scripts/download-model.sh zipformer-small        Download the LiteRT Zipformer small model
  ./scripts/download-model.sh zipformer-medium       Download the LiteRT Zipformer medium model
  ./scripts/download-model.sh zipformer-large        Download the LiteRT Zipformer large model
  ./scripts/download-model.sh seallms-audio-7b --yes  Download the experimental SeaLLMs files
  ./scripts/download-model.sh --list                 List available model artifacts
  ./scripts/download-model.sh --help                 Show this help

Downloads are opt-in per model. They never enable Cargo features or change the
runtime selected by the CLI/server.

SeaLLMs-Audio is a download-only experiment at present. It is approximately
16.6 GB, has an `other/seallms` licence, and has no native Pheme VA adapter.
The explicit --yes flag is required before downloading it.
EOF
}

list_models() {
    cat <<'EOF'
Available model artifacts:
  whisper            Whisper large-v3-turbo GGML model; supported by whisper.cpp
  zipformer-small    LiteRT Zipformer small FP16 CTC model; optional Rust adapter
  zipformer-medium   LiteRT Zipformer medium FP16 CTC model; optional Rust adapter
  zipformer-large    LiteRT Zipformer large FP16 CTC model; optional Rust adapter
  seallms-audio-7b   SeaLLMs-Audio 7B safetensors; download-only, requires --yes
EOF
}

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
    local path=$1
    local expected=$2
    local label=$3
    local actual

    if [[ -z "$expected" ]]; then
        printf 'download-model: no SHA-256 is recorded for %s; refusing to continue\n' "$label" >&2
        return 1
    fi

    actual=$(checksum "$path")
    if [[ "$actual" != "$expected" ]]; then
        printf 'download-model: checksum mismatch for %s\nexpected: %s\nactual:   %s\n' \
            "$label" "$expected" "$actual" >&2
        return 1
    fi
}

download_artifact() {
    local label=$1
    local url=$2
    local expected=$3
    local destination=$4
    local part_path="${destination}.part"

    mkdir -p "$(dirname "$destination")"

    if [[ -f "$destination" ]]; then
        printf 'Verifying existing %s\n' "$label"
        verify "$destination" "$expected" "$label"
        printf 'Already present and verified: %s\n' "$destination"
        return 0
    fi

    if [[ -f "$part_path" ]]; then
        # A completed .part file can be promoted without another network call.
        if verify "$part_path" "$expected" "$label"; then
            mv "$part_path" "$destination"
            printf 'Promoted verified download: %s\n' "$destination"
            return 0
        fi
        printf 'Resuming partial %s: %s\n' "$label" "$part_path"
    else
        printf 'Downloading %s to %s\n' "$label" "$destination"
    fi

    printf 'Progress is shown below; the partial file is kept until verification succeeds.\n'
    curl --fail --show-error --location --retry 3 --retry-all-errors \
        --continue-at - --progress-bar \
        --output "$part_path" "$url"
    printf '\n'

    verify "$part_path" "$expected" "$label"
    mv "$part_path" "$destination"
    printf 'Downloaded and verified: %s\n' "$destination"
}

download_whisper() {
    download_artifact \
        "Whisper large-v3-turbo model" \
        "$WHISPER_URL" \
        "$WHISPER_SHA256" \
        "$MODEL_DIR/whisper/ggml-large-v3-turbo.bin"
}

download_zipformer() {
    local variant=$1
    local source_file
    local model_sha256

    case "$variant" in
        small)
            source_file="zipformer_ctc_small_fp16.tflite"
            model_sha256="$ZIPFORMER_SMALL_SHA256"
            ;;
        medium)
            source_file="zipformer_ctc_fp16.tflite"
            model_sha256="$ZIPFORMER_MEDIUM_SHA256"
            ;;
        large)
            source_file="zipformer_ctc_large_fp16.tflite"
            model_sha256="$ZIPFORMER_LARGE_SHA256"
            ;;
        *)
            printf 'download-model: unknown Zipformer variant: %s\n' "$variant" >&2
            return 2
            ;;
    esac

    download_artifact \
        "LiteRT Zipformer ${variant} BPE model" \
        "$LITERT_BASE_URL/bpe.model" \
        "$BPE_SHA256" \
        "$MODEL_DIR/zipformer/bpe.model"
    download_artifact \
        "LiteRT Zipformer ${variant} token list" \
        "$LITERT_BASE_URL/tokens.txt" \
        "$TOKENS_SHA256" \
        "$MODEL_DIR/zipformer/tokens.txt"
    download_artifact \
        "LiteRT Zipformer ${variant} model" \
        "$LITERT_BASE_URL/$source_file" \
        "$model_sha256" \
        "$MODEL_DIR/zipformer/$variant/model.tflite"

    printf 'LiteRT Zipformer %s artifacts are ready under %s/zipformer/%s\n' \
        "$variant" "$MODEL_DIR" "$variant"
    printf 'Note: downloading files does not select a model or activate a Cargo feature.\n'
}

download_seallms() {
    printf '%s\n' \
        'WARNING: SeaLLMs-Audio-7B is an experimental download-only artifact.' \
        'It is approximately 16.6 GB, uses the upstream `other/seallms` licence,' \
        'and cannot currently be loaded by a native Pheme VA backend.' \
        'The files are kept under models/seallms-audio-7b/.'

    download_artifact \
        "SeaLLMs-Audio model shard 1/4" \
        "$SEALLMS_BASE_URL/model-00001-of-00004.safetensors" \
        "$SEALLMS_MODEL_1_SHA256" \
        "$MODEL_DIR/seallms-audio-7b/model-00001-of-00004.safetensors"
    download_artifact \
        "SeaLLMs-Audio model shard 2/4" \
        "$SEALLMS_BASE_URL/model-00002-of-00004.safetensors" \
        "$SEALLMS_MODEL_2_SHA256" \
        "$MODEL_DIR/seallms-audio-7b/model-00002-of-00004.safetensors"
    download_artifact \
        "SeaLLMs-Audio model shard 3/4" \
        "$SEALLMS_BASE_URL/model-00003-of-00004.safetensors" \
        "$SEALLMS_MODEL_3_SHA256" \
        "$MODEL_DIR/seallms-audio-7b/model-00003-of-00004.safetensors"
    download_artifact \
        "SeaLLMs-Audio model shard 4/4" \
        "$SEALLMS_BASE_URL/model-00004-of-00004.safetensors" \
        "$SEALLMS_MODEL_4_SHA256" \
        "$MODEL_DIR/seallms-audio-7b/model-00004-of-00004.safetensors"
    download_artifact \
        "SeaLLMs-Audio tokenizer" \
        "$SEALLMS_BASE_URL/tokenizer.json" \
        "$SEALLMS_TOKENIZER_SHA256" \
        "$MODEL_DIR/seallms-audio-7b/tokenizer.json"
}

selected_model=""
allow_seallms=false
for argument in "$@"; do
    case "$argument" in
        --help|-h)
            usage
            exit 0
            ;;
        --list)
            list_models
            exit 0
            ;;
        --yes)
            allow_seallms=true
            ;;
        whisper|zipformer-small|zipformer-medium|zipformer-large|seallms-audio-7b)
            if [[ -n "$selected_model" ]]; then
                printf 'download-model: choose one model at a time\n' >&2
                usage >&2
                exit 2
            fi
            selected_model=$argument
            ;;
        *)
            printf 'download-model: unknown argument: %s\n' "$argument" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [[ -z "$selected_model" ]]; then
    selected_model=whisper
fi

case "$selected_model" in
    whisper)
        download_whisper
        ;;
    zipformer-small)
        download_zipformer small
        ;;
    zipformer-medium)
        download_zipformer medium
        ;;
    zipformer-large)
        download_zipformer large
        ;;
    seallms-audio-7b)
        if [[ "$allow_seallms" != true ]]; then
            printf 'download-model: SeaLLMs requires the explicit --yes flag\n' >&2
            usage >&2
            exit 2
        fi
        download_seallms
        ;;
esac
