# Local model files

Model weights and runtime artifacts are intentionally ignored by Git. Download
only the model family you want to evaluate; there is no default download-all
operation.

## Model status

| Name                     | Artifact format        | Current Pheme VA status                              | License/source                               |
| ------------------------ | ---------------------- | ---------------------------------------------------- | -------------------------------------------- |
| `whisper-large-v3-turbo` | whisper.cpp GGML       | Supported by the optional `whispercpp` crate         | See upstream repository and model terms      |
| `zipformer-small`        | LiteRT/TFLite FP16 CTC | Supported by the optional `zipformer` crate          | Apache-2.0, pinned LiteRT Community revision |
| `zipformer-medium`       | LiteRT/TFLite FP16 CTC | Supported by the optional `zipformer` crate          | Apache-2.0, pinned LiteRT Community revision |
| `zipformer-large`        | LiteRT/TFLite FP16 CTC | Supported by the optional `zipformer` crate          | Apache-2.0, pinned LiteRT Community revision |
| `seallms-audio-7b`       | BF16 safetensors       | Download-only experiment; no native Pheme VA adapter | `other/seallms`; check upstream terms        |

A downloaded file does **not** activate a Cargo feature, select a model, or
make an unsupported runtime loadable. Runtime selection belongs in the host
CLI/server configuration after an adapter has been implemented and validated.
The core's `Transcriber` trait is the Lego-style boundary: each model crate
accepts normalized mono 16 kHz audio and returns the same `RawTranscription`
contract. The CLI resolves model IDs through `manifest.toml`; downloading an
artifact does not activate a Cargo feature.

## Whisper baseline

- File: `whisper/ggml-large-v3-turbo.bin`
- Source: <https://huggingface.co/ggerganov/whisper.cpp>
- Pinned revision: `5359861c739e955e79d9a303bcbc70fb988958b1`
- Download URL: `https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo.bin`
- Size: approximately 1.62 GB
- SHA-256: `1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69`
- Runtime: optional in-process `whisper-rs`/whisper.cpp adapter

Download and verify it with either command:

```bash
./scripts/download-model.sh
./scripts/download-model.sh whisper
```

Use it from the Pheme VA workspace:

```bash
cargo run --release -p cli --features whisper -- \
  --stt-model whisper-large-v3-turbo tui
```

## LiteRT Zipformer CTC models

These three English ASR models use the same interface and are
interchangeable through the same `ZipformerTranscriber`. The upstream contract
is:

```text
16 kHz mono PCM
  -> host Kaldi-compatible 80-bin fbank features
  -> LiteRT inputs [1, 1600, 80] and masks [1, 796], [1, 398], [1, 199], [1, 100]
  -> CTC logits [1, 398, 500]
  -> host greedy CTC decoding and BPE detokenization
```

- Repository: <https://huggingface.co/litert-community/Zipformer-medium-CR-CTC-LiteRT>
- Pinned revision: `7732ad6c15ec43402968d5ae04acfa7a204027f5`
- License: Apache-2.0
- Shared `bpe.model` SHA-256: `c53433de083c4a6ad12d034550ef22de68cec62c4f58932a7b6b8b2f1e743fa5`
- Shared `tokens.txt` SHA-256: `49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb`

| Variant            | Model file                        | Approx. size | SHA-256                                                            |
| ------------------ | --------------------------------- | -----------: | ------------------------------------------------------------------ |
| `zipformer-small`  | `zipformer_ctc_small_fp16.tflite` |        46 MB | `ff6d70c7e8cfdfcf994be625456ca6543e0dbcc76fd4fd428e2138642d0852ba` |
| `zipformer-medium` | `zipformer_ctc_fp16.tflite`       |       132 MB | `9515d94c04306798fecfe695eb8f79cc8ac98ed3d8eab81c28ac464b7dbddf48` |
| `zipformer-large`  | `zipformer_ctc_large_fp16.tflite` |       298 MB | `183c928cd1b109ad0b94d9540dbf4c2660393428d2104494b6047af4aa4eb1da` |

Download one variant; the script resumes `.part` files and only promotes a
file after checksum verification:

```bash
./scripts/download-model.sh zipformer-small
# or: zipformer-medium / zipformer-large
```

Files are stored as:

```text
models/zipformer/bpe.model
models/zipformer/tokens.txt
models/zipformer/<variant>/model.tflite
```

The Rust workspace includes the optional `zipformer` crate. Build the CLI with
`--features zipformer` (or both `whisper zipformer`) to activate that model
family; downloaded files alone do not change the selected CLI model.

The model IDs and paths are recorded in [`manifest.toml`](manifest.toml).

## SeaLLMs-Audio experimental artifact

SeaLLMs-Audio is a multimodal audio-language model, not a Whisper-compatible
ASR checkpoint. It is approximately 16.6 GB and currently has no supported
native Rust/LiteRT/whisper.cpp adapter in Pheme VA. The downloader is therefore
explicitly download-only and requires `--yes`:

```bash
./scripts/download-model.sh seallms-audio-7b --yes
```

- Repository: <https://huggingface.co/SeaLLMs/SeaLLMs-Audio-7B>
- Pinned revision: `c5c3152b373350653ec6dd981d041f8285ab26c9`
- License metadata: `other`, `seallms` (do not assume permissive redistribution)

| File                               | Approx. size | SHA-256                                                            |
| ---------------------------------- | -----------: | ------------------------------------------------------------------ |
| `model-00001-of-00004.safetensors` |      4.93 GB | `6ab818c2f728fc9030bfe149c3ed666a8770bba1cedb18a9dc81e1db64213a21` |
| `model-00002-of-00004.safetensors` |      4.99 GB | `8fb2dc90d06f8392dcc9f9f0fb01cdc88e86d531babcccd04c18ce0c136295a1` |
| `model-00003-of-00004.safetensors` |      4.93 GB | `472cead59645d4f981b9f8cac931e5a334ad1b0b3baf794016df203ec43b6992` |
| `model-00004-of-00004.safetensors` |      1.72 GB | `e50c202a880e492c974d982d4b802d28477618dd65f2eea08849ac522ad4cee1` |
| `tokenizer.json`                   |        12 MB | `fecdb47d281073055efd605d080013e3114ed0f3c5d8af201e245b199864c9c7` |

Do not download this model as part of normal setup. A real adapter would need
a supported native inference runtime, multimodal processor, prompt contract,
and a memory/performance evaluation before it could implement the core
`Transcriber` interface.

## Integrity and Git policy

Every artifact uses a pinned URL/revision and SHA-256 verification. Network
failures and failed checksums leave the `.part` file in place so a later run
can resume or inspect it. Model files are ignored by Git; do not commit model
weights, recordings, or generated benchmark artifacts. Check each upstream
model's licence and terms before redistributing it.
