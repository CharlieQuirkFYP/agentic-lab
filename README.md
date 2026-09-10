# Agentic Lab

## Goal

Agentic Lab is a university final year project in collaboration with KLASS. The team is building its own agentic voice-based incident reporting solution, with spoken clarification, human confirmation, and retrieval of previous reports.

The working AI baseline is Whisper speech recognition and local language-model inference through llama.cpp, with speech synthesis completing the spoken interaction. The research evaluates quality, latency, resource usage, power consumption, energy, and thermal/endurance trade-offs on resource-constrained hardware. A mobile phone, potentially iOS, is the preferred eventual target; NVIDIA Jetson is an alternative pending device specifications.

The planned `voice-agent/` Python service owns the complete incident workflow and its runtime adapters. Go exposes the public API and coordinates benchmarks.

The web app will provide a development voice console and a benchmark dashboard. Existing KLASS solutions will not be integrated. Vision/licence-plate monitoring is out of scope; interview development is deferred.

## Project Documentation

* [Requirements and scope](docs/requirements.md)
* [Planned system architecture](docs/architecture.md)
* [Benchmark methodology](docs/benchmark-methodology.md)
* [Implementation roadmap and ticket breakdown](docs/roadmap.md)
* [Planned Voice Agent API contract](docs/api/voice-agent.md)
* [Current experiment API](docs/api/experiments.md)
* [Contributor/agent instructions](AGENTS.md)

## Local Development

The following instructions cover Go, Voice Agent, and the web scaffold. Model/runtime setup will be added when inference is implemented.

### Go API

#### Prerequisites

- Go 1.25.0 installed
- Git
- The repository cloned locally

#### Setup

From the repository root, navigate to the API service:

```bash
cd api
```

Synchronize and install dependencies:

```bash
go mod tidy
```

Start the API:

```bash
go run ./cmd/server
```

The server runs at:

```text
http://localhost:8080
```

#### Verify the API

Check the health endpoint:

```bash
curl http://localhost:8080/health
```

Expected response:

```json
{
  "status": "ok"
}
```

Check the incident analysis endpoint:

```bash
curl -X POST \
  http://localhost:8080/api/v1/incidents/analyze \
  -H "Content-Type: application/json" \
  -d '{
    "transcript": "A vehicle collided with a barrier near the west entrance."
  }'
```

Expected response:

```json
{
  "incident_type": "unknown",
  "location": "unknown",
  "severity": "unknown",
  "summary": "A vehicle collided with a barrier near the west entrance.",
  "recommended_action": "Pending AI analysis"
}
```

The incident analysis response is currently a placeholder; AI integration has not yet been implemented.

#### Testing

Run the test suite:

```bash
go test ./...
```

Run static checks:

```bash
go vet ./...
```

### Voice Agent

Requires Python 3.12+. From the repository root:

```bash
cd voice-agent
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r requirements-dev.lock
python -m pip install --no-deps --no-build-isolation -e .
python -m uvicorn app.main:create_app --factory --host 127.0.0.1 --port 8000
```

`GET /health` returns 200. With the default unavailable backend, readiness and analysis return 503. Follow [local inference setup](voice-agent/docs/local-analysis.md) to enable llama.cpp-backed analysis. The Go API still uses its mock. See [Voice Agent setup and checks](voice-agent/README.md) for configuration and verification.

### Web

#### Prerequisites

- Node.js 20.19+ or 22.12+
- npm

#### Setup

From the repository root:

```bash
cd web
npm install
```

Start the frontend:

```bash
npm run dev
```

The Vite dev server runs at:

```text
http://localhost:5173
```

#### Verification

Build the frontend:

```bash
npm run build
```

Run the configured lint command:

```bash
npm run lint
```
