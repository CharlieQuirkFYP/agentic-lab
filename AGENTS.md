# Agentic Lab — Codex Instructions

## Project Purpose

Agentic Lab is a university final year project developed in collaboration with KLASS.

The project focuses on building and evaluating agentic AI workflows across different hardware environments, especially low-spec and resource-constrained environments.

The main research goal is to compare trade-offs in:

* latency
* memory usage
* CPU/GPU usage
* throughput
* task success
* output quality

The project may either:

1. implement reference AI workflows ourselves, or
2. integrate existing KLASS solutions and adapt them for low-spec deployment and benchmarking.

The architecture should remain flexible enough to support either path.

## Use Case Priority

The current development priority is:

1. Voice-Based Incident Reporting
2. Interview Understanding and Intelligence
3. Vision-Based Monitoring / Licence Plate Detection

Do not scaffold later use cases unless explicitly requested.

Focus development on the incident reporting use case first.

## Current Architecture Direction

The intended high-level architecture is:

```text
Frontend
   ↓
Go API
   ↓
Implementation / Integration Boundary
   ↓
Python AI Service or KLASS Solution
   ↓
Local Model / Runtime where required
   ↓
Benchmarking and Evaluation
```

The Go API is the application-facing backend.

Future AI-heavy functionality is expected to live primarily in Python.

The Go API should not become tightly coupled to a specific AI implementation.

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

Future implementations may include:

```text
IncidentAnalyzer
├── PythonIncidentAnalyzer
└── KlassIncidentAnalyzer
```

Do not bypass the analyzer boundary by calling AI implementations directly from handlers.

## Architecture Principles

Prefer simple, explicit, maintainable architecture.

Follow these rules:

* Keep HTTP concerns inside handlers.
* Keep application logic inside services.
* Keep request/response/domain structs inside model packages.
* Use interfaces at integration boundaries where multiple implementations may exist.
* Avoid interfaces for every struct purely for abstraction.
* Do not tightly couple the Go backend to Gemma, llama.cpp, LangGraph, or any KLASS-specific implementation.
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
* incident handler/service/model separation
* `IncidentAnalyzer` interface
* `MockIncidentAnalyzer`
* unit tests for incident analyzer and service
* frontend toolchain scaffold under `web/`
* basic local Go setup instructions in `README.md`
* basic local Web setup instructions in `README.md`

Not yet implemented:

* Python AI service
* Gemma
* llama.cpp
* LangGraph
* ASR / faster-whisper
* interview workflow
* vision workflow
* KLASS adapters
* benchmarking runtime
* PostgreSQL
* Redis
* Kafka
* Docker
* frontend product pages

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

The expected progression is roughly:

```text
Go API
   ↓
Python AI service / KLASS integration
   ↓
local model runtime
   ↓
benchmarking
   ↓
database / frontend
   ↓
async infrastructure only if needed
```

Redis may be introduced later for caching or job state.

Kafka may be introduced later if distributed benchmark execution or event-driven workflows genuinely require it.

Do not add either simply because they appear in the project proposal.

## AI Implementation Guidance

The AI implementation strategy is not yet finalized.

Possible path A:

```text
Go API
   ↓
Python AI Service
   ↓
LangGraph / ASR / CV
   ↓
llama.cpp
   ↓
Gemma or other SLM
```

Possible path B:

```text
Go API
   ↓
KLASS Adapter
   ↓
Existing KLASS solution
   ↓
local runtime / model adaptation where required
```

A hybrid of both may also be used.

When implementing new components, preserve the ability to support both student-built and KLASS-provided implementations where reasonable.

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

`web/` and `ai-service/` are future components and should not be created before they are needed.

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

Potential future documentation includes:

* architecture
* benchmark methodology
* use-case specifications
* KLASS integration
* deployment

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
