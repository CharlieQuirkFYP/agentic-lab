# Local model files

Model weights are intentionally ignored by Git. The current local STT baseline is:

- File: `ggml-large-v3-turbo.bin`
- Source: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin`
- Size: approximately 1.62 GB
- SHA-256: `1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69`

The model is a Whisper speech-recognition model. It is not the same thing as
`voice-agent/llama.cpp-revision`, which is only a pinned llama.cpp source
revision. A llama.cpp language model is a separate future cleanup/extraction
model and is not currently bundled here.

Use the model from the Pheme VA workspace:

```bash
cargo run --release -p cli --features whisper -- \
  tui --model models/ggml-large-v3-turbo.bin
```

Do not commit model weights. Check the upstream model licence and terms before
redistributing them.
