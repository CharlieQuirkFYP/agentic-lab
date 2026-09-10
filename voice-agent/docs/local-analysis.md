# Local Incident Analysis

Voice Agent can now analyze text using the pinned llama.cpp HTTP server. Enable the adapter explicitly; the default `unavailable` backend still makes no runtime calls. This is stateless extraction, not the speech/conversation workflow. Go still uses its mock analyzer until T05.

## Provisional Model

The integration smoke test uses the official [Qwen3-0.6B GGUF](https://huggingface.co/Qwen/Qwen3-0.6B-GGUF), Q8_0, under Apache-2.0. It is a small development probe, not final model selection or evidence of production-quality incident understanding. MERaLiON/Gemma and target-device suitability remain evaluation decisions.

Pinned model repository revision: `23749fefcc72300e3a2ad315e1317431b06b590a`.

File: `Qwen3-0.6B-Q8_0.gguf` (about 610 MiB).

SHA-256: `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`.

Download once outside Git (skip if already downloaded and verified):

```bash
mkdir -p "$HOME/.local/share/agentic-lab/models"
curl -fL --retry 2 \
  https://huggingface.co/Qwen/Qwen3-0.6B-GGUF/resolve/23749fefcc72300e3a2ad315e1317431b06b590a/Qwen3-0.6B-Q8_0.gguf \
  -o "$HOME/.local/share/agentic-lab/models/Qwen3-0.6B-Q8_0.gguf"
shasum -a 256 "$HOME/.local/share/agentic-lab/models/Qwen3-0.6B-Q8_0.gguf"
```

Compare the checksum above before loading. Keep the upstream model licence with any redistributed weights. No model download occurs in ordinary tests or Python startup.

## Run Both Processes

Install the [pinned runtime](llama-cpp.md) and [Python dependencies](../README.md) first. From `voice-agent/`, start llama.cpp in terminal 1:

```bash
export VOICE_AGENT_MODEL_PATH="$HOME/.local/share/agentic-lab/models/Qwen3-0.6B-Q8_0.gguf"
export VOICE_AGENT_LLAMA_CONTEXT=4096
./scripts/start-llama.sh
```

In terminal 2, also from `voice-agent/`:

```bash
source .venv/bin/activate
export VOICE_AGENT_ANALYZER=llama_cpp
export VOICE_AGENT_LLAMA_CPP_URL=http://127.0.0.1:8081
python -m uvicorn app.main:create_app --factory --host 127.0.0.1 --port 8000
```

Stop both processes with Ctrl+C when done. Voice Agent does not launch, download, or select models itself. Python's legacy `VOICE_AGENT_MODEL_PATH` setting does not load a model; the llama.cpp launcher owns loading.

Check and submit an incident in another terminal:

```bash
curl -i http://127.0.0.1:8000/ready
curl -i http://127.0.0.1:8000/v1/incidents/analyze \
  -H 'Content-Type: application/json' \
  -d '{"transcript":"A vehicle collided with a barrier at the west entrance. The incident severity is low. Please notify the site supervisor."}'
```

Readiness returns 200 when llama.cpp reports `status: ok`. Success returns the existing five report fields without a wrapper. Try `There is smoke.` and the ambiguous example from the [contract](../../docs/api/voice-agent.md). Inspect the facts, not just the HTTP status. Keep these development examples separate from a future held-out evaluation set.

With llama.cpp stopped, readiness and valid analysis return 503, while `/health` remains 200. Readiness establishes model loading/availability, not extraction quality or compatibility with every runtime option.

## Adapter Behaviour

1. Read `/props` for actual per-slot context size, model filename, and build identifier.
2. Apply the model's chat template to the versioned system prompt, development examples, and JSON-encoded transcript; tokenize the resulting prompt.
3. Reserve configured output tokens plus a 32-token margin. Reject insufficient context without truncating the transcript or attempting generation.
4. Send one non-streaming `/v1/chat/completions` request with schema-constrained fields/types, temperature 0, seed 0, and thinking disabled. No retries or automatic repair.
5. Require a single completion with `finish_reason: stop`; reject token-limited output, tool calls/refusals, malformed JSON, and invalid report fields.
6. Validate all report lengths and types in Python before returning success.

The pinned runtime rejected grammars expanded from the full report string-length limits. Therefore generation constrains structure/types/enums, while Python enforces the complete schema including lengths. This is intentional and covered by tests. Schema validity does not prove factual grounding.

The extra template/tokenization calls add preflight overhead. The context check is conservative and assumes the pinned single-model server; router mode and runtime swaps during a request are outside this integration. Revalidate templates/schema support when changing runtime or model.

The asynchronous HTTP client is reused and closed by application lifespan. Proxy environment variables are ignored for these local runtime calls. Existing application deadlines and disconnect cancellation propagate to the pending HTTP operation; runtime cancellation need not immediately preempt accelerator work.

## Configuration and Errors

| Variable | Default | Meaning |
| --- | --- | --- |
| `VOICE_AGENT_ANALYZER` | `unavailable` | `llama_cpp` enables real inference; `unavailable` makes no runtime calls |
| `VOICE_AGENT_LLAMA_CPP_URL` | `http://127.0.0.1:8081` | Root URL of the pinned single-model server, without `/v1` |
| `VOICE_AGENT_ANALYSIS_TIMEOUT_SECONDS` | `55` | Overall operation budget, including preflight and inference |
| `VOICE_AGENT_MAX_OUTPUT_TOKENS` | `512` | Generation cap, 64–4096; truncated output is rejected |

Connection/protocol failures and upstream 429/502/503 map to `runtime_unavailable` (503). Upstream timeouts/504 and overall deadline expiration map to `analysis_timeout` (504). Malformed success envelopes or invalid report output map to `invalid_model_response` (502). Other upstream errors/configuration mismatches and insufficient context map to `analysis_failed` (500), retaining the existing contract. Runtime response bodies are not forwarded.

For context failures, lower the output cap or deliberately increase the runtime context if the model/hardware supports it. The API's 16,000-character validation limit does not guarantee every input fits the runtime token budget. The adapter never silently truncates it.

## Execution Traces

Uvicorn logs one JSON `incident_analysis` event per attempted extraction. It includes the prompt version (`incident-extraction-v1`), backend, runtime build, model filename, actual context size, estimated prompt tokens, output cap, sampling settings, outcome, overall duration, and generation duration when available.

`duration_ms` covers adapter preflight, generation, and validation; `generation_ms` covers the completion HTTP call. These are wall-clock durations, not device energy or model-only compute measurements. Cancellation is recorded as cancelled; a service deadline cancelling the adapter can appear as cancelled in the trace while the public API returns 504. `success` means structurally valid output, not independently verified task quality.

Traces omit transcripts, generated reports, raw exceptions, and full model paths. Filename/build identity alone is insufficient for reproducibility: retain the model revision/checksum and launcher/backend settings above with experiment results. Persistent benchmark ingestion is future work.

## Smoke-Test Evidence and Limits

Verified on 2026-09-10 with the model/revision/checksum above, llama.cpp v0.4.0 commit `5266f24`, Darwin arm64 CPU build, 4 threads, zero GPU offload, and 4096 context tokens. This was a development smoke test, not a repeated benchmark or held-out evaluation.

| Input | Final observed result |
| --- | --- |
| Complete collision example in the contract | 200; collision, west entrance, explicitly stated low severity, requested supervisor notification |
| `There is smoke.` | 200; smoke report, unknown location/severity/action, summary retains smoke |
| Ambiguous east-or-west-gate collision example | 200; possible collision, unknown location/severity/action, uncertainty retained in summary |
| `Water is leaking from a pipe near the north stairwell.` | 200 and valid schema, but **incorrectly categorized as smoke report**; location and water-leak summary retained |

The prompt was refined using development examples after initial smoke/ambiguity errors. The final water-leak error is retained as a known limitation, not silently fixed through a test-specific rule. This tiny provisional model is not an accepted quality baseline. A wider held-out dataset and model comparison are needed before deployment claims. The runtime and API now function, but structural validity alone is not task success.
