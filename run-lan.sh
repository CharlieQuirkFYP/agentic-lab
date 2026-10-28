#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if command -v hostname >/dev/null 2>&1; then
  LAN_IP="$(hostname -I 2>/dev/null | awk '{print $1}')"
fi

if [[ -z "${LAN_IP:-}" ]]; then
  echo "Could not determine the computer's LAN IP address."
  echo "Run 'hostname -I', then start Expo manually with:"
  echo 'EXPO_PUBLIC_API_BASE_URL="http://YOUR_LAN_IP:8080/api/v1" npx expo start --lan -c'
  exit 1
fi

if [[ ! -f "$ROOT/ios-app/package.json" ]]; then
  echo "Expo project not found: $ROOT/ios-app/package.json"
  exit 1
fi

echo "Using LAN IP: $LAN_IP"
echo "Backend: http://$LAN_IP:8080"
echo "Pheme:   http://$LAN_IP:8000"
echo "Dashboard: http://$LAN_IP:5173/dashboard"

cleanup() {
  echo "Stopping services..."
  kill 0 2>/dev/null || true
}

trap cleanup EXIT INT TERM

# Start the Pheme voice server.
(
  cd "$ROOT/pheme-va"
  cargo run --release -p server --features whisper -- \
    --model models/transcript/whisper/ggml-large-v3-turbo.bin \
    --language en \
    --reply-model qwen2.5-1.5b-instruct-q4-k-m \
    --metrics-enabled \
    --resource-sampling \
    --bind 0.0.0.0:8000
) &

# Start the Go API on the LAN so the Expo phone client can reach it.
(
  cd "$ROOT/api"
  API_ADDR="0.0.0.0:8080" \
  API_CORS_ORIGINS="http://localhost:5173,http://127.0.0.1:5173,http://${LAN_IP}:5173" \
  PHEME_VA_URL="http://127.0.0.1:8000" \
  go run ./cmd/server
) &

# Start the benchmark dashboard. Vite proxies /api/v1 to the Go API.
(
  cd "$ROOT/web"
  npm run dev -- --host 0.0.0.0
) &

# Start Expo over LAN.
(
  cd "$ROOT/ios-app"
  EXPO_PUBLIC_API_BASE_URL="http://${LAN_IP}:8080/api/v1" \
    npx expo start --lan -c
) &

wait
