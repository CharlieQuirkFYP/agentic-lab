#!/usr/bin/env bash
# Offline downloader integration tests. Fake curl/checksum tools use tiny fixtures.
set -euo pipefail
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
SCRIPT="$SCRIPT_DIR/download-model.sh"
TEST_DIR=$(mktemp -d)
active_pid=""
cleanup() {
    if [[ -n "$active_pid" ]]; then
        kill "$active_pid" 2>/dev/null || true
        wait "$active_pid" 2>/dev/null || true
    fi
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT
mkdir -p "$TEST_DIR/bin"
export PHEME_VA_MODEL_DIR="$TEST_DIR/models"
export FAKE_LOG="$TEST_DIR/curl.log"
export FAKE_STARTED="$TEST_DIR/started"
export PATH="$TEST_DIR/bin:$PATH"

cat > "$TEST_DIR/bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_LOG"
destination=""
previous=""
for arg in "$@"; do
    if [[ "$previous" == --output ]]; then destination="$arg"; fi
    previous="$arg"
done
url="${!#}"
case "$url" in
    */ggml-large-v3-turbo.bin) fixture=whisper ;;
    */bpe.model) fixture=bpe ;;
    */tokens.txt) fixture=tokens ;;
    */zipformer_ctc_small_fp16.tflite) fixture=small ;;
    */zipformer_ctc_fp16.tflite) fixture=medium ;;
    */zipformer_ctc_large_fp16.tflite) fixture=large ;;
    */qwen2.5-1.5b-instruct-q4_k_m.gguf) fixture=qwen ;;
    */model-00001-of-00004.safetensors) fixture=sea1 ;;
    */model-00002-of-00004.safetensors) fixture=sea2 ;;
    */model-00003-of-00004.safetensors) fixture=sea3 ;;
    */model-00004-of-00004.safetensors) fixture=sea4 ;;
    */tokenizer.json) fixture=seatok ;;
    *) exit 99 ;;
esac
if [[ "${FAKE_CURL_MODE:-ok}" == slow ]]; then
    printf 'partial' > "$destination"
    touch "$FAKE_STARTED"
    # No grandchildren: the downloader's signal must terminate the writer.
    trap 'exit 143' TERM INT
    while true; do read -r -t 0.1 unused || true; done
fi
if [[ "${FAKE_CURL_MODE:-ok}" == fail ]]; then
    printf 'partial' > "$destination"
    exit 22
fi
if [[ "${FAKE_CURL_MODE:-ok}" == corrupt ]]; then fixture=corrupt; fi
printf '%s' "$fixture" > "$destination"
EOF
cat > "$TEST_DIR/bin/sha256sum" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
fixture=$(cat "$1")
case "$fixture" in
    whisper) sha=1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69 ;;
    bpe) sha=c53433de083c4a6ad12d034550ef22de68cec62c4f58932a7b6b8b2f1e743fa5 ;;
    tokens) sha=49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb ;;
    small) sha=ff6d70c7e8cfdfcf994be625456ca6543e0dbcc76fd4fd428e2138642d0852ba ;;
    medium) sha=9515d94c04306798fecfe695eb8f79cc8ac98ed3d8eab81c28ac464b7dbddf48 ;;
    large) sha=183c928cd1b109ad0b94d9540dbf4c2660393428d2104494b6047af4aa4eb1da ;;
    qwen) sha=6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e ;;
    sea1) sha=6ab818c2f728fc9030bfe149c3ed666a8770bba1cedb18a9dc81e1db64213a21 ;;
    sea2) sha=8fb2dc90d06f8392dcc9f9f0fb01cdc88e86d531babcccd04c18ce0c136295a1 ;;
    sea3) sha=472cead59645d4f981b9f8cac931e5a334ad1b0b3baf794016df203ec43b6992 ;;
    sea4) sha=e50c202a880e492c974d982d4b802d28477618dd65f2eea08849ac522ad4cee1 ;;
    seatok) sha=fecdb47d281073055efd605d080013e3114ed0f3c5d8af201e245b199864c9c7 ;;
    *) sha=bad ;;
esac
printf '%s  %s\n' "$sha" "$1"
EOF
chmod +x "$TEST_DIR/bin/curl" "$TEST_DIR/bin/sha256sum"

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
expect_fail() {
    if bash "$SCRIPT" "$@" > "$TEST_DIR/failure.log" 2>&1; then fail "unexpected success: $*"; fi
}
reset_root() {
    rm -rf "$PHEME_VA_MODEL_DIR"
    rm -f "$FAKE_LOG" "$FAKE_STARTED"
    unset FAKE_CURL_MODE
}

bash "$SCRIPT" --help > "$TEST_DIR/help"
bash "$SCRIPT" --list > "$TEST_DIR/list"
grep -q 'qwen2.5-1.5b-instruct-q4-k-m' "$TEST_DIR/list"
expect_fail 'untrusted;command'
expect_fail whisper zipformer-small
expect_fail seallms-audio-7b
[[ ! -e "$FAKE_LOG" && ! -d "$PHEME_VA_MODEL_DIR" ]] || fail 'invalid arguments/consent touched model files'

# No-argument Whisper default, safe migration (legacy files untouched).
mkdir -p "$PHEME_VA_MODEL_DIR/whisper"
printf 'user-owned legacy weights' > "$PHEME_VA_MODEL_DIR/whisper/ggml-large-v3-turbo.bin"
bash "$SCRIPT" > "$TEST_DIR/output"
[[ -f "$PHEME_VA_MODEL_DIR/transcript/whisper/ggml-large-v3-turbo.bin" ]] || fail 'wrong Whisper destination'
grep -q 'user-owned legacy weights' "$PHEME_VA_MODEL_DIR/whisper/ggml-large-v3-turbo.bin"
cp "$FAKE_LOG" "$TEST_DIR/before"
bash "$SCRIPT" whisper > "$TEST_DIR/output"
cmp "$FAKE_LOG" "$TEST_DIR/before" || fail 'verified existing file caused network call'
printf 'bad existing weights' > "$PHEME_VA_MODEL_DIR/transcript/whisper/ggml-large-v3-turbo.bin"
expect_fail whisper
grep -q 'bad existing weights' "$PHEME_VA_MODEL_DIR/transcript/whisper/ggml-large-v3-turbo.bin"
cmp "$FAKE_LOG" "$TEST_DIR/before" || fail 'bad existing file was overwritten'

# All STT arguments; shared bundle files are verified/reused.
reset_root
for variant in small medium large; do
    bash "$SCRIPT" "zipformer-$variant" > "$TEST_DIR/output"
    [[ -f "$PHEME_VA_MODEL_DIR/transcript/zipformer/$variant/model.tflite" ]] || fail "wrong Zipformer $variant destination"
done
[[ $(wc -l < "$FAKE_LOG") == 5 ]] || fail 'shared Zipformer tokenizer files redownloaded'

# Pinned reply and unchanged SeaLLMs consent/dispatcher.
bash "$SCRIPT" qwen2.5-1.5b-instruct-q4-k-m > "$TEST_DIR/output"
[[ -f "$PHEME_VA_MODEL_DIR/reply/qwen2.5-1.5b-instruct-q4_k_m.gguf" ]] || fail 'wrong reply destination'
grep -q 'Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/91cad51170dc346986eccefdc2dd33a9da36ead9' "$FAKE_LOG"
bash "$SCRIPT" --yes seallms-audio-7b > "$TEST_DIR/output"
[[ -f "$PHEME_VA_MODEL_DIR/seallms-audio-7b/model-00004-of-00004.safetensors" ]] || fail 'SeaLLMs legacy argument changed'

# Complete partial files promote offline; failed transfers resume using curl's
# existing --continue-at - semantics, only after checksum verification.
reset_root
mkdir -p "$PHEME_VA_MODEL_DIR/reply"
printf qwen > "$PHEME_VA_MODEL_DIR/reply/qwen2.5-1.5b-instruct-q4_k_m.gguf.part"
bash "$SCRIPT" qwen2.5-1.5b-instruct-q4-k-m > "$TEST_DIR/output"
[[ ! -e "$FAKE_LOG" ]] || fail 'verified partial caused network call'
reset_root
export FAKE_CURL_MODE=fail
expect_fail whisper
[[ -f "$PHEME_VA_MODEL_DIR/transcript/whisper/ggml-large-v3-turbo.bin.part" ]] || fail 'network failure discarded partial'
[[ ! -f "$PHEME_VA_MODEL_DIR/transcript/whisper/ggml-large-v3-turbo.bin" ]] || fail 'network failure promoted partial'
unset FAKE_CURL_MODE
bash "$SCRIPT" whisper > "$TEST_DIR/output" 2>&1
grep -q -- '--continue-at -' "$FAKE_LOG"
[[ ! -e "$PHEME_VA_MODEL_DIR/.download.lock" ]] || fail 'normal exit retained lock'
reset_root
export FAKE_CURL_MODE=corrupt
expect_fail whisper
[[ -f "$PHEME_VA_MODEL_DIR/transcript/whisper/ggml-large-v3-turbo.bin.part" && ! -f "$PHEME_VA_MODEL_DIR/transcript/whisper/ggml-large-v3-turbo.bin" ]] || fail 'checksum failure promoted/discarded partial'

# Root lock covers different models and shared files. Cancellation waits for
# the native writer, retains partial bytes, and permits explicit retry.
reset_root
export FAKE_CURL_MODE=slow
bash "$SCRIPT" whisper > "$TEST_DIR/slow.log" 2>&1 &
active_pid=$!
for attempt in {1..100}; do
    [[ -e "$FAKE_STARTED" ]] && break
    sleep 0.05
done
[[ -e "$FAKE_STARTED" ]] || fail 'fake downloader did not start'
expect_fail zipformer-small
grep -q 'another writer' "$TEST_DIR/failure.log"
[[ $(wc -l < "$FAKE_LOG") == 1 ]] || fail 'concurrent process reached curl'
kill -TERM "$active_pid"
wait "$active_pid" 2>/dev/null || true
active_pid=""
[[ ! -e "$PHEME_VA_MODEL_DIR/.download.lock" ]] || fail 'cancel did not release lock'
[[ -f "$PHEME_VA_MODEL_DIR/transcript/whisper/ggml-large-v3-turbo.bin.part" ]] || fail 'cancel discarded partial'
unset FAKE_CURL_MODE
bash "$SCRIPT" whisper > "$TEST_DIR/output" 2>&1
printf 'PASS: offline model downloader arguments, bundles, pins, integrity, resume, cancellation and writer exclusion\n'
