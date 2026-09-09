# Agentic Lab Web

React + TypeScript + Vite frontend scaffold for Agentic Lab. Only the toolchain and placeholder home page are currently implemented.

## Planned Roles

* Development voice console: record speech, play responses, and inspect transcripts, report drafts, and conversation state.
* Benchmark dashboard: configure experiments, monitor status, inspect results, compare model/device configurations, and export results.

The operational incident workflow prioritizes speech; the research dashboard remains a visual interface. Follow the existing light, restrained engineering-dashboard conventions. See the [architecture](../docs/architecture.md) and [implementation roadmap](../docs/roadmap.md).

The current Go API supports experiment creation and lookup by ID. Listing, persistent results, comparisons, and real measurements are planned backend work, not existing frontend capabilities.

## Development

```bash
npm install
npm run dev
```

## Verification

```bash
npm run build
npm run lint
```
