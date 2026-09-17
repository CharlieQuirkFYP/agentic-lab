# Voice Agent Web Console

Standalone, phone-friendly development frontend for the Pheme VA HTTP host. It does not modify the benchmark dashboard under `web/`.

## Run locally

1. Start Pheme VA on port 8000 from `../pheme-va/` (see its README for model setup).
2. Install and start this frontend:

```bash
npm install
npm run dev
```

Vite proxies `/pheme/*` to `http://127.0.0.1:8000/*`, avoiding development-browser CORS issues. To use a different host, set `VITE_PHEME_API_URL` before starting Vite.

The browser captures a completed microphone recording, encodes it as WAV, calls `/v1/transcribe`, then sends non-empty transcript text to `/v1/analyze`. The current Pheme service has no streaming endpoint, so live transcription is intentionally out of scope here.

For a physical phone, access the Vite server over HTTPS: browsers permit microphone access only in secure contexts (with `localhost` as a development exception). A deployed version also needs a same-origin reverse proxy or CORS configured by the Pheme host.
