#!/usr/bin/env bash
# Start the external llama.cpp server; this does not start the Python service.
set -euo pipefail

fail() { printf 'start-llama: %s\n' "$*" >&2; exit 1; }
usage() {
    cat <<'HELP'
Usage: start-llama.sh [--check | --dry-run | --help]

  --check    Run the executable's --version (no model required).
  --dry-run  Validate paths/settings and print the launch command without running it.
  --help     Show this help.

Environment:
  VOICE_AGENT_LLAMA_SERVER_BIN  Executable path (default: external v0.4.0 build)
  VOICE_AGENT_MODEL_PATH        Required readable local model file for launch/dry-run
  VOICE_AGENT_LLAMA_HOST        Bind address (default: 127.0.0.1)
  VOICE_AGENT_LLAMA_PORT        Port 1–65535 (default: 8081)
  VOICE_AGENT_LLAMA_CONTEXT     Context tokens (default: 4096)
  VOICE_AGENT_LLAMA_THREADS     CPU threads (default: 4)
  VOICE_AGENT_LLAMA_GPU_LAYERS  Non-negative offload count (default: 0, CPU baseline)

No downloads, model selection, or automatic .env loading are performed.
See docs/llama-cpp.md for pinned installation and model-backed verification.
HELP
}

[[ $# -le 1 ]] || fail 'Expected at most one option; use --help.'
mode=${1-}
case "$mode" in
    --help) usage; exit 0 ;;
    ''|--check|--dry-run) ;;
    *) fail "Unknown option: $mode (use --help)." ;;
esac

server=${VOICE_AGENT_LLAMA_SERVER_BIN-"${HOME}/.local/share/agentic-lab/llama.cpp-v0.4.0/build/bin/llama-server"}
[[ -f "$server" && -x "$server" ]] || fail "Executable not found or not executable: $server. Set VOICE_AGENT_LLAMA_SERVER_BIN to a built llama-server."
if [[ "$mode" == --check ]]; then
    exec "$server" --version
fi

model=${VOICE_AGENT_MODEL_PATH-}
[[ -n "$model" ]] || fail 'Set VOICE_AGENT_MODEL_PATH to an external compatible model file.'
[[ -f "$model" && -r "$model" ]] || fail "Model is not a readable regular file: $model"
host=${VOICE_AGENT_LLAMA_HOST-127.0.0.1}
[[ -n "$host" && "$host" != -* && ! "$host" =~ [[:space:]] ]] || fail 'VOICE_AGENT_LLAMA_HOST must be a non-empty bind address without whitespace.'
port=${VOICE_AGENT_LLAMA_PORT-8081}
context=${VOICE_AGENT_LLAMA_CONTEXT-4096}
threads=${VOICE_AGENT_LLAMA_THREADS-4}
gpu_layers=${VOICE_AGENT_LLAMA_GPU_LAYERS-0}

# Bound the string length before arithmetic to prevent overflow or shell expressions.
number() {
    local name=$1 value=$2 minimum=$3 maximum=$4
    [[ "$value" =~ ^[0-9]{1,9}$ ]] || fail "$name must be an integer from $minimum to $maximum."
    (( 10#$value >= minimum && 10#$value <= maximum )) || fail "$name must be an integer from $minimum to $maximum."
}
number VOICE_AGENT_LLAMA_PORT "$port" 1 65535
number VOICE_AGENT_LLAMA_CONTEXT "$context" 1 1000000
number VOICE_AGENT_LLAMA_THREADS "$threads" 1 1024
number VOICE_AGENT_LLAMA_GPU_LAYERS "$gpu_layers" 0 1000000

command=("$server" --model "$model" --host "$host" --port "$port"
    --ctx-size "$context" --threads "$threads" --n-gpu-layers "$gpu_layers")
if [[ "$mode" == --dry-run ]]; then
    printf '%q ' "${command[@]}"
    printf '\n'
    exit 0
fi
# Replace the shell so Ctrl+C and exit status reach the runtime directly.
exec "${command[@]}"
