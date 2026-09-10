# Local model files

Model weights are intentionally ignored by Git. The current local STT baseline is:

- File: `ggml-large-v3-turbo.bin`
- Source: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin`
- Size: approximately 1.62 GB
- SHA-256: `1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69`

The model is a Whisper speech-recognition model. It is used by Pheme VA's
in-process `whisper-rs` backend, which is built on whisper.cpp. It is not a
language model for incident extraction; a separate local language-model
adapter remains a future evaluation target.

Download and verify the ignored model with:

```bash
./scripts/download-model.sh
```

Use the model from the Pheme VA workspace:

```bash
cargo run --release -p cli --features whisper -- \
  tui --model models/ggml-large-v3-turbo.bin
```

Do not commit model weights. Check the upstream model licence and terms before
redistributing them.
