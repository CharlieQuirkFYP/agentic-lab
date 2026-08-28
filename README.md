# Agentic Lab
## Goal
This is a university final year project in collaboration with Klass, focused on building and evaluating agentic AI workflows across different hardware environments.

The project explores how lightweight AI agents can perform practical tasks using local models, speech recognition, retrieval, and computer vision, while measuring the trade-offs between performance, resource usage, and output quality.

## Local Development

Local setup instructions will be expanded as more services are introduced. For now, only the initial Go API setup is documented.

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
