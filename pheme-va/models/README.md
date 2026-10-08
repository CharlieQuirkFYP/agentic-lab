# Local model files

Model weights and runtime artifacts are intentionally ignored by Git. Download
only the model family you want to evaluate; there is no default download-all
operation.

## Model status

| Name                           | Artifact format        | Current Pheme VA status                                        | License/source                               |
| ------------------------------ | ---------------------- | -------------------------------------------------------------- | -------------------------------------------- |
| `whisper-large-v3-turbo`       | whisper.cpp GGML       | Supported by the optional `whispercpp` crate                   | MIT, pinned Hugging Face repository          |
| `zipformer-small`              | LiteRT/TFLite FP16 CTC | Supported by the optional `zipformer` crate                    | Apache-2.0, pinned LiteRT Community revision |
| `zipformer-medium`             | LiteRT/TFLite FP16 CTC | Supported by the optional `zipformer` crate                    | Apache-2.0, pinned LiteRT Community revision |
| `zipformer-large`              | LiteRT/TFLite FP16 CTC | Supported by the optional `zipformer` crate                    | Apache-2.0, pinned LiteRT Community revision |
| `qwen2.5-1.5b-instruct-q4-k-m` | GGUF Q4_K_M            | Persistent local llama.cpp worker; normal `reply-model` client | Apache-2.0, official Qwen repository         |
| `seallms-audio-7b`             | BF16 safetensors       | Download-only experiment; no native Pheme VA adapter           | `other/seallms`; check upstream terms        |

A downloaded file does **not** activate a Cargo feature, select a model, or
make an unsupported runtime loadable. Runtime selection belongs in the host
CLI/server configuration after an adapter has been implemented and validated.
The core's `Transcriber` trait is the Lego-style boundary: each model crate
accepts normalized mono 16 kHz audio and returns the same `RawTranscription`
contract. The CLI resolves model IDs through `manifest.toml`; downloading an
artifact does not activate a Cargo feature.

## Whisper baseline

- File: `transcript/whisper/ggml-large-v3-turbo.bin`
- Source: <https://huggingface.co/ggerganov/whisper.cpp>
- Pinned revision: `5359861c739e955e79d9a303bcbc70fb988958b1`
- Download URL: `https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo.bin`
- Size: 1,624,555,275 bytes (approximately 1.62 decimal GB)
- License metadata: MIT, verified through the pinned Hugging Face revision
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
models/transcript/zipformer/bpe.model
models/transcript/zipformer/tokens.txt
models/transcript/zipformer/<variant>/model.tflite
```

The Rust workspace includes the optional `zipformer` crate. Build the CLI with
`--features zipformer` (or both `whisper zipformer`) to activate that model
family; downloaded files alone do not change the selected CLI model.

The model IDs, paths and exact model byte sizes are recorded in
[`manifest.toml`](manifest.toml). The pinned upstream API reports 46,216,688,
131,490,944 and 297,947,504 bytes for small, medium and large respectively;
these are model-file sizes, excluding the shared tokenizer artifacts.

## Qwen native reply baseline

This is a small instruction-tuned **text reply** model, not a transcriber or
cleanup model. It is a development baseline, not a validated mobile/device
configuration or a guarantee of incident-reporting quality.

- Catalog/download ID: `qwen2.5-1.5b-instruct-q4-k-m`
- File: `reply/qwen2.5-1.5b-instruct-q4_k_m.gguf`
- Official source: <https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF>
- Pinned revision: `91cad51170dc346986eccefdc2dd33a9da36ead9`
- Quantization: `Q4_K_M`
- Exact size: **1,117,320,736 bytes** (1.12 decimal GB)
- SHA-256: `6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e`
- License: [Apache-2.0 at the pinned revision](https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/blob/91cad51170dc346986eccefdc2dd33a9da36ead9/LICENSE)
- Metadata provenance: the [Hugging Face model API with LFS blob metadata](https://huggingface.co/api/models/Qwen/Qwen2.5-1.5B-Instruct-GGUF?blobs=true)
  reports the above commit, filename, byte size and LFS SHA-256; license text
  was checked at the pinned revision. Verification did not download the weights.
- Default role: `../roles/incident-reporting.txt`, relative to `manifest.toml` (the checked-in file is `pheme-va/roles/incident-reporting.txt`).

```bash
./scripts/download-model.sh qwen2.5-1.5b-instruct-q4-k-m
```

The `reply-native` crate pins `llama-cpp-2` and the matching sys binding to
`0.1.158`. It builds `pheme-reply-worker`, a **host-owned persistent stdio
child**, not another HTTP service. Whisper and llama.cpp ship incompatible
GGML symbols, so linking both directly into one host is unsafe. The normal
`reply-model` dependency implements the same trait over bounded, versioned
NDJSON; the worker alone links llama.cpp. There is no enable-reply Cargo feature.
Building the worker requires a C/C++ toolchain, CMake and libclang for bindgen.
Build/ship both executables together:

```bash
cargo build --release -p cli -p server -p reply-native --features cli/whisper,server/whisper
```

Each TUI/server host finds `pheme-reply-worker` beside its own executable; an explicit
trusted `PHEME_VA_REPLY_WORKER` path can override that location. Runtime startup
never invokes Cargo. A missing worker fails before a reply download is attempted.
The host starts/stops/reaps its worker; clients never manage a second service.
Cancellation reaches the child's native atomic through an independent input
thread. An unresponsive/crashed worker is killed/reaped and answering becomes
not-ready until an explicit host restart, without a transcription fallback.
Ordinary cancelled/completed turns keep the same loaded weights.

Resource accounting must include the native child PID. The current desktop
sampler covers the hosting process, not its reply child; reply-specific CPU/RAM
metrics therefore remain explicitly unavailable rather than understated.
The default dependency disables optional accelerator/OpenMP/common-library
features; use `gpu_layers=0` with this CPU build. Nonzero GPU layers fail clearly
when no GPU backend is compiled rather than silently falling back.

`ReplyModel::load(path, config, threads, gpu_layers)` loads weights once and
prewarms the native compute path with a disposable context. Each
`ConversationModel::respond_stream` call uses a fresh KV context and sampler,
formats the separate system/user/assistant messages through the GGUF's own
chat template, and streams only complete UTF-8 chunks. Input/context, output
characters and tokens are bounded. Atomic cancellation is checked before/after
native work and installed as llama.cpp's native abort callback (CPU graph
execution can abort during decode). No cancelled/failed reply should be committed
as successful history by the host. Native accelerator abort responsiveness can
vary; measure it before deploying an accelerated build.

The shared registry lives in `va_core::registry` and is re-exported by
`va_core` and the CLI's `model` module. `ModelEntry::load_prompt` loads the
manifest default; `load_prompt_files` combines 1–32 non-empty UTF-8 regular
files in selected order, preserving each source's exact text and inserting
exact `\n\n` separators. The combined text, including separators, is bounded
by `MAX_PROMPT_CHARS` (16,384 Unicode scalar values); its exact UTF-8 bytes
produce the preview/turn SHA-256. This does not increase the existing default
12,000-character aggregate role + history + question budget; startup rejects
a role consuming that entire budget. Missing/invalid roles never fall back
to generic instructions. Trusted overrides and edits apply only after a
TUI/server restart. The prompt does
not authorize saving, dispatch, retrieval or other tools.

The unified TUI Models page derives both purpose groups from this manifest.
Use `o` on a reply row to browse `.txt` files beside its default role or add
another trusted local path, and `p` to preview the ordered combined text/hash.
Available reply rows do not load a local runtime on Enter. See
[Models controls and role/startup precedence](../README.md#unified-models-page).

Normal tests use fakes/tiny fixtures and never fetch weights. The ignored
adapter smoke test takes both model and prompt paths from the environment:

```bash
PHEME_VA_REPLY_MODEL=/absolute/path/to/model.gguf \
PHEME_VA_REPLY_PROMPT=/absolute/path/to/incident-reporting.txt \
  cargo test -p reply-model real_worker_streams_without_linking_llama_into_client -- --ignored
```

## Local registry and path migration

All catalog entries keep their existing IDs and `[[models]]` format. Explicit
`purpose` separates `transcript` from `reply`; only known legacy Whisper and
Zipformer families default to transcription. `download_id` handles the old
Whisper catalog ID versus script argument. Artifact resolution accepts both
manifest-relative paths and legacy `models/`-prefixed workspace paths;
`system_prompt` is always manifest-directory-relative.

New downloads install STT bundles under `models/transcript/` and instruction
weights under `models/reply/`. **Neither the registry nor the downloader moves,
deletes or overwrites your existing legacy weights.** To reuse files already
under `models/whisper/` or `models/zipformer/`, either keep explicit legacy paths
in local configuration/custom manifests, or deliberately copy/move them yourself
to the new layout after checking the destinations. Zipformer requires its
`bpe.model` and `tokens.txt` alongside all selected variant directories. Existing
path-based host arguments remain valid for files at their original locations.
Do not run a new download expecting it to discover old paths automatically.

`PHEME_VA_MODEL_DIR` overrides the downloader's artifact root, not the manifest
or prompt directory. For a custom root, use a corresponding local manifest or
explicit host paths. Serializable
`StartupChoices {stt_model, reply_model, reply_prompt_files}` records IDs and
an optional ordered role-path list for the **next start**, not active readiness.
The additive list defaults to empty for older choices files; relative paths
resolve from the choices file's directory. TUI config keeps per-model absolute
paths in `reply_role_files`, and exports the chosen reply model's list. Saving
choices never loads models, restarts the server or initiates a download.

Repeated server `--prompt-file` flags combine sources in flag order and conflict
with the legacy single-file `--system-prompt`. Explicit prompt flags override
saved roles; saved roles apply only to their matching reply model ID. Selecting
a different explicit ID uses that entry's manifest default unless prompt flags
are supplied. Raw `--reply-path` requires explicit prompts and never inherits
saved roles. With no saved/explicit role choice, a selected catalog reply model
uses its manifest `system_prompt`.

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
can resume or inspect it. Existing final files are verified, never overwritten.
A completed valid `.part` promotes without network access. A corrupt complete
partial may require deliberate removal after inspection; resume cannot repair
arbitrary incorrect bytes.

A portable atomic `models/.download.lock/` prevents concurrent writers across
the whole artifact root, including shared Zipformer files. Normal exit and
cancellation release it only after the curl child exits. After SIGKILL or a
crash, a stale lock is not stolen automatically: verify that no downloader or
curl writer is running before removing it. Cancellation retains partial data.
Run offline downloader coverage with `bash scripts/test-download-model.sh`.

Model files are ignored by Git; do not commit model
weights, recordings, or generated benchmark artifacts. Check each upstream
model's licence and terms before redistributing it.
