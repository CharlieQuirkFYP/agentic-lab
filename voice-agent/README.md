# Voice Agent

Python service for Voice-Based Incident Reporting. The scaffold implements the [text-analysis HTTP contract](../docs/api/voice-agent.md) with an injectable analyzer boundary. Model inference, sessions, speech, and storage are not implemented yet. Go still uses its mock analyzer and does not call this service.

## Local Development

Requires Python 3.12+ (CI uses 3.12). From this directory:

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r requirements-dev.lock
python -m pip install --no-deps --no-build-isolation -e .
python -m uvicorn app.main:create_app --factory --host 127.0.0.1 --port 8000
```

`pyproject.toml` declares dependency ranges; `requirements-dev.lock` pins the tested development/CI environment. Update the lock deliberately when changing dependencies. No model weights or network access are required to run the tests or service after dependency installation.

## Verify

```bash
curl -i http://127.0.0.1:8000/health
curl -i http://127.0.0.1:8000/ready
curl -i http://127.0.0.1:8000/v1/incidents/analyze \
  -H 'Content-Type: application/json' \
  -d '{"transcript":"Smoke reported near the west entrance."}'
```

Health returns `200 {"status":"ok"}`. Readiness and valid analysis requests return `503` with `runtime_unavailable` until a real adapter is wired in T04. Invalid analysis requests return the documented 400/415 errors. The default analyzer never invents a report. Tests inject small local fakes to verify success, failures, timeout, and cancellation.

The OpenAPI UI is at `http://127.0.0.1:8000/docs`.

## Configuration

Environment variables are read when the application factory starts. Invalid settings fail startup.

| Variable | Default | Purpose |
| --- | --- | --- |
| `VOICE_AGENT_ANALYSIS_TIMEOUT_SECONDS` | `55` | Positive finite budget for analysis and readiness checks |
| `VOICE_AGENT_LLAMA_CPP_URL` | `http://127.0.0.1:8081` | Validated runtime URL reserved for T04; not contacted by the scaffold |
| `VOICE_AGENT_MODEL_PATH` | unset | Optional external model path reserved for T04; not loaded or checked for existence yet |

Host and port use Uvicorn command-line options. `.env` files are not automatically loaded. Keep the Python budget below the future Go client's timeout. Async runtime adapters must cooperate with cancellation; cancelling an HTTP request does not guarantee immediate accelerator preemption.

## Checks

```bash
python -m ruff format --check .
python -m ruff check .
python -m pytest
```

## Layout

* `app/main.py`: application factory, HTTP validation, error mapping, disconnect handling.
* `app/config.py`: environment configuration.
* `app/incident_reporting/models.py`: strict request/report schemas.
* `app/incident_reporting/service.py`: analyzer protocol, unavailable implementation, timeout and output validation.
* `tests/`: contract/configuration tests using fakes; no model downloads.

The workflow, repositories, and runtime adapter directory will be added as their tickets are implemented.

## Local llama.cpp Setup

See [pinned runtime installation and launcher](docs/llama-cpp.md). The runtime is installed outside this repository and started with `scripts/start-llama.sh`; `--check` verifies the executable without a model. Final model selection and model-backed inference verification remain deferred. Python does not connect to this server until the adapter ticket is implemented.
