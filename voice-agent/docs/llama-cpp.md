# Local llama.cpp Runtime

This setup ticket installs llama.cpp separately from Python and provides a launcher. It does not select a final SLM, download weights, or connect Voice Agent to inference. For the subsequent Python integration and provisional-model smoke test, see [local analysis](local-analysis.md).

## Version Pin

Use upstream release **v0.4.0**, peeled commit **5266f24da75dc449bd56cbed7addb9c8e4a6a73e**, recorded in [llama.cpp-revision](../llama.cpp-revision). The tag is annotated; the tag object's SHA differs from the checkout commit. Verify the commit rather than relying on a moving branch or release label alone.

References: [pinned source](https://github.com/ggml-org/llama.cpp/tree/5266f24da75dc449bd56cbed7addb9c8e4a6a73e), [build instructions](https://github.com/ggml-org/llama.cpp/blob/v0.4.0/docs/build.md), and [server API/options](https://github.com/ggml-org/llama.cpp/blob/v0.4.0/tools/server/README.md).

## Install Outside the Repository

Prerequisites: Git, CMake, a C/C++ compiler, and a build tool (Make for these commands). On macOS, install the Xcode Command Line Tools if absent (`xcode-select --install`). Use your package manager for CMake if needed. The initial build is CPU-only; accelerator selection/optimization is deferred. This does not establish iOS or Jetson deployment compatibility.

From `voice-agent/`, in Bash or Zsh:

```bash
LLAMA_SOURCE_DIR="$HOME/.local/share/agentic-lab/llama.cpp-v0.4.0"
LLAMA_PINNED_REVISION="$(cat llama.cpp-revision)"
git clone --depth 1 --branch v0.4.0 \
  https://github.com/ggml-org/llama.cpp.git "$LLAMA_SOURCE_DIR"
test "$(git -C "$LLAMA_SOURCE_DIR" rev-parse HEAD)" = "$LLAMA_PINNED_REVISION"
```

Stop if the comparison fails. For an existing installation, skip cloning and verify the revision and clean source state (`git -C "$LLAMA_SOURCE_DIR" status --short`). Do not reset or overwrite an existing checkout with local changes.

```bash
cmake -S "$LLAMA_SOURCE_DIR" -B "$LLAMA_SOURCE_DIR/build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DLLAMA_BUILD_IS_DEV=OFF \
  -DLLAMA_BUILD_TESTS=OFF \
  -DLLAMA_BUILD_EXAMPLES=OFF \
  -DLLAMA_BUILD_APP=OFF \
  -DLLAMA_BUILD_SERVER=ON \
  -DLLAMA_BUILD_UI=OFF \
  -DLLAMA_USE_PREBUILT_UI=OFF \
  -DLLAMA_OPENSSL=OFF \
  -DGGML_METAL=OFF \
  -DGGML_CUDA=OFF
cmake --build "$LLAMA_SOURCE_DIR/build" --target llama-server --parallel 4
./scripts/start-llama.sh --check
```

This builds only the requested server target and its dependencies. HTTPS support and bundled UI downloads are disabled for this local HTTP setup. Retain the build's `bin/` directory and libraries together; do not copy only the executable. `--check` executes `--version`; compare the output with the pinned checkout. It does not load a model or start an HTTP listener.

### macOS SDK Header Troubleshooting

On the development machine, Clang's default C++ include search missed headers present in the SDK (`cstdio`/`cstddef` not found). If this exact issue occurs, verify that the header exists and configure an explicit include path before rebuilding:

```bash
LLAMA_MACOS_SDK="$(xcrun --show-sdk-path)"
test -f "$LLAMA_MACOS_SDK/usr/include/c++/v1/cstdio"
cmake -S "$LLAMA_SOURCE_DIR" -B "$LLAMA_SOURCE_DIR/build" \
  "-DCMAKE_CXX_FLAGS=-isystem \"$LLAMA_MACOS_SDK/usr/include/c++/v1\""
cmake --build "$LLAMA_SOURCE_DIR/build" --target llama-server --parallel 4
```

This changes only the external build configuration. The flag replaces existing `CMAKE_CXX_FLAGS`; preserve any custom flags if adapting a pre-existing build. It is not needed on a correctly configured toolchain.

## Configure and Launch

Choose a compatible local model only when ready to test inference. Place weights outside the repository. The launcher checks file accessibility, not GGUF integrity, model architecture support, chat templates, or available memory.

```bash
export VOICE_AGENT_MODEL_PATH="/absolute/path/outside/repository/model.gguf"
./scripts/start-llama.sh --dry-run
./scripts/start-llama.sh
```

| Variable | Default | Meaning |
| --- | --- | --- |
| `VOICE_AGENT_LLAMA_SERVER_BIN` | `$HOME/.local/share/agentic-lab/llama.cpp-v0.4.0/build/bin/llama-server` | Executable file path; quote paths containing spaces |
| `VOICE_AGENT_MODEL_PATH` | required for launch/dry-run | Readable external model file |
| `VOICE_AGENT_LLAMA_HOST` | `127.0.0.1` | Bind address; keep local for this development setup |
| `VOICE_AGENT_LLAMA_PORT` | `8081` | Port, 1–65535 |
| `VOICE_AGENT_LLAMA_CONTEXT` | `4096` | Context tokens, 1–1,000,000; runtime/model limits still apply |
| `VOICE_AGENT_LLAMA_THREADS` | `4` | CPU threads, 1–1024 |
| `VOICE_AGENT_LLAMA_GPU_LAYERS` | `0` | Offloaded layers, 0–1,000,000; requires a separately validated accelerator-enabled build |

The defaults are a small development starting point, not optimized settings. The context includes the prompt and generated output; it is not a guarantee that the Voice Agent contract's full 16,000-character input fits. The adapter now preflights the formatted prompt against the runtime context; see [local analysis](local-analysis.md).

`.env` files are not automatically loaded. Environment values are passed as literal arguments, without shell evaluation. Existing upstream `LLAMA_ARG_*` variables can affect options not explicitly set by the launcher; use a clean runtime environment and record all overrides during experiments. The launcher does not accept arbitrary extra CLI arguments.

`--dry-run` validates configuration and prints a shell-escaped command without executing it. `--check` needs only the executable, not a model. Invalid paths, numeric values, and unknown options fail with a `start-llama:` error. Stop a launched server with Ctrl+C; the script uses `exec`, preserving runtime signals and exit status. If the port is occupied, the runtime reports the bind failure; change the port or stop the process you own.

The Python service remains a separate process on port 8000. Its `VOICE_AGENT_LLAMA_CPP_URL` defaults to `http://127.0.0.1:8081`; with `VOICE_AGENT_ANALYZER=llama_cpp`, set that URL to match any launcher host/port changes. This launcher does not alter Python configuration or Go's mock analyzer.

## Verification Levels

1. **Installation:** verify the pinned source commit, successfully build, and run `--check`/`llama-server --help`. No model required.
2. **Launcher:** run the automated fake-executable tests and a dry-run with an existing file. This proves argument handling, not inference.
3. **Model-backed smoke test (deferred):** after choosing a provisional compatible model and starting the server, check readiness and generation:

```bash
curl -i http://127.0.0.1:8081/health
curl -i http://127.0.0.1:8081/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"messages":[{"role":"user","content":"Reply with a short greeting."}],"max_tokens":32,"stream":false}'
```

Wait for loading to complete; health should return 200, and generation should return a valid completion. Record the model identity/checksum, quantization, runtime commit, build/backend, and launch settings. A successful greeting is a runtime smoke test, not incident-quality evaluation. Do not claim this level passed without a model-backed run.

## Maintenance

Update the revision file, release/default executable path, these build instructions, and verification evidence together when changing the runtime pin. Normal Python CI runs launcher tests using a fake executable and no weights. Rebuild/version checks are explicit runtime-maintenance work, not an automatic dependency download during tests.

## Historical T04a Setup Verification

Built on Darwin arm64 with CMake 4.1.2 and AppleClang 21.0.0, using the CPU-only flags above and the documented SDK header workaround. The external source checkout matches the revision file and has no source changes.

`start-llama.sh --check` reported:

```text
version: 0.4.0 (build 1, commit 5266f24)
built with AppleClang 21.0.0.21000101 for Darwin arm64
```

The installed executable's help includes every launcher flag. Launcher argument/error tests pass without model weights. No model was selected/downloaded, no inference request was executed, and no runtime server was left running. Model-backed health/generation verification remains deferred.
