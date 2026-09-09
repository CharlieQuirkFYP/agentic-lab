# Agentic Lab — Codex Instructions

## Project Purpose

Agentic Lab is a university final year project developed in collaboration with KLASS.

The team will build its own agentic speech-to-speech incident reporting solution and evaluate efficient local deployment on resource-constrained hardware. Existing KLASS solutions and KLASS adapters are not an implementation path.

The research compares power consumption, energy per completed task, battery endurance, thermal behaviour, latency, memory, CPU/GPU usage, throughput, task success, and output quality.

## Use Case Scope

* Voice-Based Incident Reporting is the active implementation scope, including clarification, correction, human confirmation, report storage, and spoken retrieval/summarization.
* Interview Understanding and Intelligence is outside the current development focus; it has not explicitly been removed.
* Vision-Based Monitoring / Licence Plate Detection (Use Case 3) is removed.

Do not scaffold other use cases. The operational interaction should prioritize speech. The web app remains both a development voice console and a benchmark dashboard.

## Architecture Direction

The planned development architecture is:

```text
Web voice console / benchmark dashboard
   ↓
Go API — application logic, session state, confirmation, repositories
   ↓
Python AI service — transcription, interpretation, extraction, summarization
   ↓
Whisper speech recognition + llama.cpp language-model runtime
```

Speech output uses a replaceable speech-synthesis adapter, initially at the client. Benchmark workers exercise the same application workflow and collect measurements. SQLite is planned when session/report persistence is implemented, followed by persistent benchmark results.

The Go API remains the application-facing backend. Keep AI-heavy functionality behind integration interfaces; do not couple handlers to a model runtime. Go owns authoritative conversation state and validates AI-proposed actions. Start with an explicit state machine; LangGraph is not required for the initial workflow.

The preferred eventual target is a mobile phone, potentially an iOS device supplied by KLASS; NVIDIA Jetson is an alternative. Hardware specifications and selection are pending. Build the local prototype now. Do not assume the Go/Python service layout ships unchanged inside an iOS app, or that remote inference accessed from a phone qualifies as on-device inference.

See [requirements](docs/requirements.md), [architecture](docs/architecture.md), [benchmark methodology](docs/benchmark-methodology.md), and [implementation roadmap](docs/roadmap.md). These distinguish planned work from implemented behaviour.

## Current Go API

The Go API lives under:

```text
api/
```

It currently uses:

* Go
* Gin
* handler / service / model separation
* analyzer interfaces for AI implementation boundaries

Existing endpoints:

```text
GET /health

POST /api/v1/incidents/analyze

POST /api/v1/experiments

GET /api/v1/experiments/:id
```

The incident endpoint currently returns a placeholder response through `MockIncidentAnalyzer`.

The current dependency flow is:

```text
IncidentHandler
      ↓
IncidentService
      ↓
IncidentAnalyzer
      ↓
MockIncidentAnalyzer
```

This abstraction is intentional.

The planned real implementation is `PythonIncidentAnalyzer`. Retain the mock for isolated development and tests. Add multi-turn conversation logic separately rather than forcing it into the one-shot `Analyze` contract.

Do not bypass the analyzer boundary by calling AI implementations directly from handlers.

## Current Benchmark Lifecycle

Benchmark experiments are asynchronous. The current code still accepts these legacy use-case values:

* Incident Reporting
* Interview Assistant
* License Plate Monitoring

This describes current API behaviour, not the active project scope. Removing the licence-plate value requires an explicit code/test/API documentation change; a documentation update alone does not remove support.

The current implementation lives inside the Go API under:

```text
api/internal/benchmark/
```

Experiment creation stores a queued experiment in memory, enqueues a small job containing the experiment ID, and returns immediately. A background Go worker consumes jobs from an in-memory buffered channel, marks experiments as running, executes the configured `Runner`, stores the result, and marks the experiment as completed or failed.

The current runner is mocked and does not call real AI workflows, external services, or model runtimes.

Repository, queue, worker, and runner implementations are intentionally replaceable. Planned work adds a real incident runner, SQLite-backed experiment storage, listing/filtering APIs, and dashboard comparisons. Retain the in-memory queue and internal worker until a concrete requirement justifies replacing them. Current benchmark values are hardcoded, not measurements.

## Architecture Principles

Prefer simple, explicit, maintainable architecture.

Follow these rules:

* Keep HTTP concerns inside handlers.
* Keep application logic inside services.
* Keep request/response/domain structs inside model packages.
* Use interfaces at integration boundaries where multiple implementations may exist.
* Avoid interfaces for every struct purely for abstraction.
* Do not tightly couple the Go backend to a particular model, Whisper implementation, llama.cpp, or orchestration library.
* Preserve public API contracts unless a task explicitly requires changing them.
* Prefer incremental development over scaffolding the entire future architecture.
* Do not create services or infrastructure before they solve an actual requirement.
* Avoid premature microservice decomposition.

## Go Conventions

For Go code:

* Prefer idiomatic, readable Go.
* Keep abstractions minimal.
* Use `context.Context` for service/integration calls where cancellation or timeouts may matter.
* Use constructors where dependencies need to be injected.
* Wire concrete dependencies in `cmd/server/main.go`.
* Do not introduce dependency injection frameworks.
* Use `gofmt`.
* Run `go vet`.
* Keep error responses simple and avoid exposing internal error details over HTTP.

## Frontend Conventions

The web app has two planned roles: a voice workflow console for development and a research dashboard for experiment creation, progress, results, comparisons, and export. A speech-first operational use case does not remove the dashboard requirement. Neither role is implemented beyond the toolchain scaffold.

For frontend code:

* Use React + TypeScript + Vite.
* Use React Router DOM for routing.
* Use Tailwind CSS.
* Use shadcn/ui for reusable UI primitives.
* Use Lucide React for icons.
* Prefer simple reusable components.
* Prefer a clean engineering/research dashboard aesthetic.
* Default to light mode unless explicitly changed.
* Use neutral surfaces, subtle borders, and restrained accents.
* Prioritize experiment/data readability.
* Remain responsive.
* Avoid excessive gradients, glassmorphism, animation, and decorative effects.
* Avoid turning the product into a marketing/SaaS landing page.
* Avoid adding frontend dependencies unless they solve a clear requirement.

## Testing Conventions

For Go unit/package tests:

* Keep `*_test.go` files beside the source package they test.
* Use Go's standard `testing` package by default.
* Avoid mocking frameworks unless clearly necessary.
* Prefer small local fakes/stubs for interface testing.

A future top-level:

```text
tests/
```

directory may be used for:

* cross-service integration tests
* Go ↔ Python tests
* full benchmark pipeline tests
* end-to-end tests

Do not move normal Go unit tests into a central test directory.

## CI Guidance

GitHub Actions PR CI currently runs separate backend and frontend workflows.

The Go backend workflow checks formatting with `gofmt -l .`, runs `go vet ./...`, and runs `go test ./...` from `api/`.

The frontend workflow installs npm dependencies, runs lint, and runs the production build from `web/`.

Expand CI incrementally as new components or requirements are introduced.

## Current Implementation State

Currently implemented:

* initial Go Gin API
* `GET /health`
* `POST /api/v1/incidents/analyze`
* `POST /api/v1/experiments`
* `GET /api/v1/experiments/:id`
* incident handler/service/model separation
* `IncidentAnalyzer` interface
* `MockIncidentAnalyzer`
* generic asynchronous benchmark experiment lifecycle
* in-memory benchmark repository and job queue
* background Go benchmark worker
* mocked benchmark runner
* unit tests for incident analyzer and service
* unit tests for benchmark repository, service, worker, and experiment HTTP handler
* frontend toolchain scaffold under `web/`
* basic local Go setup instructions in `README.md`
* basic local Web setup instructions in `README.md`

Planned but not yet implemented:

* Python AI service and `PythonIncidentAnalyzer`
* Whisper transcription and llama.cpp integration with a selected local model
* speech synthesis and audio turn handling
* multi-turn sessions, clarification, corrections, and revision-bound confirmation
* SQLite session/report storage and incident retrieval
* real benchmark runner, evaluation dataset, and resource/energy measurements
* persistent benchmark results and listing/filtering APIs
* web voice console and benchmark dashboard, comparisons, and export
* target-device deployment and power/thermal/endurance evaluation

Model selection, speech-synthesis implementation, and target-device specifications remain pending. MERaLiON is an optional candidate for local speech understanding, not a required dependency. LangGraph and fine-tuning are not baseline prerequisites. Interview development is deferred; vision workflows and KLASS adapters are out of scope. PostgreSQL, Redis, Kafka, and Docker are not implemented or required by the current plan.

This section should be updated as the project evolves.

## Infrastructure Guidance

Do not add the following unless explicitly requested or justified by an actual requirement:

* Redis
* Kafka
* PostgreSQL
* Docker
* Kubernetes
* gRPC
* additional microservices

Implement incrementally: real text analysis, conversation and local persistence, speech input/output, report retrieval, real evaluation and dashboard, then target-device optimization. Start collecting stage timings with real inference. Obtain device specifications alongside implementation.

SQLite is the planned embedded persistence choice when storage is implemented. Do not add a separate database server, distributed queue, or infrastructure simply because it appeared in the original proposal.

## AI Implementation Guidance

Whisper for speech recognition and llama.cpp for language-model inference are the working baseline discussed with KLASS. The provisional Whisper runtime is whisper.cpp; exact model sizes, quantization, and runtime builds must be evaluated and recorded. Do not present this as a validated device configuration.

Keep transcription, language-model inference, and speech synthesis replaceable. Use persistent runtimes where appropriate rather than reloading weights each turn. Keep authoritative session state in Go; Python receives relevant context and returns validated proposals. Application code controls tool execution and report finalization.

Require explicit confirmation of the current draft revision before finalizing a report. Corrections invalidate prior confirmation. Retry handling must prevent duplicate turns and saved reports. Retrieve reports through validated database queries and summarize only returned records.

Build local-speech evaluation examples early. Consider vocabulary/prompt improvements, alternative models (including MERaLiON), or fine-tuning based on measured errors. Do not assume using pretrained models conflicts with building the solution ourselves.

Record unavailable measurements as unavailable, not zero. CPU utilization is not a substitute for measured power. Treat 8–10 hours as an illustrative endurance target until KLASS confirms workload and acceptance criteria.

## Repository Guidance

The intended top-level repository structure is:

```text
api/
web/
ai-service/
benchmark/
docs/
infra/
scripts/
tests/
```

`api/`, `web/`, and `docs/` exist. Create `ai-service/` when its implementation begins. Other listed directories are optional future locations, not scaffolding requirements; the benchmark lifecycle currently lives in `api/internal/benchmark/`.

Do not store large model weights or benchmark media directly in Git.

Model files such as GGUF weights should eventually live outside the source repository and be referenced through configuration or setup scripts.

Large datasets, videos, audio, and generated benchmark artifacts should also not be committed directly unless they are intentionally small fixtures.

## README vs Documentation

Keep `README.md` focused on:

* project overview
* local setup
* developer onboarding

Keep deeper documentation elsewhere, for example under:

```text
docs/
```

Project planning lives in `docs/requirements.md`, `docs/architecture.md`, `docs/benchmark-methodology.md`, and `docs/roadmap.md`. Current endpoint behaviour is documented under `docs/api/`. Add device deployment instructions once validated.

Do not overload the README with implementation history or detailed design decisions.

## Commit Style

Use Conventional Commit-style prefixes where appropriate:

```text
feat:
fix:
refactor:
docs:
test:
chore:
ci:
build:
perf:
```

Examples:

```text
feat: scaffold initial Go API

docs: add local Go API setup instructions

refactor: abstract incident analysis behind analyzer interface
```

## General Guidance

Before making changes:

1. inspect the existing repository structure
2. preserve established conventions
3. make the smallest change that satisfies the task
4. avoid introducing future infrastructure prematurely
5. update tests when behavior or architecture changes
6. run relevant formatting, tests, and static checks

When a task conflicts with this file, follow the explicit user request for that task.
