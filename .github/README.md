# Continuous integration

Use `ci-<scope>.yml` for workflow filenames and `CI / <Scope>` for Actions display
names. Each workflow runs on pull requests targeting `main` when its input paths
change, or via manual dispatch. Changes to a workflow also trigger that workflow.
Superseded runs are cancelled per workflow/ref; token permissions are read-only.

## Workflow ownership

| Workflow | Input paths | Coverage |
| --- | --- | --- |
| [CI / API](workflows/ci-api.yml) | `api/**` | Go formatting, vet and package tests. |
| [CI / Web](workflows/ci-web.yml) | `web/**` | Node fixtures, lint and production build. |
| [CI / Pheme](workflows/ci-pheme.yml) | `pheme-va/**` | Rust formatting, Clippy, workspace tests, native Whisper/reply builds and offline downloader/packaging fixtures. |
| [CI / Voice Integration](workflows/ci-voice-integration.yml) | `api/**`, `pheme-va/**`, `web/src/lib/voice/**`, `tests/**` | Real Go/Pheme HTTP boundary and web SSE handling with a fake reply worker. |
| [CI / iOS](workflows/ci-ios.yml) | `ios-app/**`, `pheme-va/**` | Expo contracts/JS bundle, Swift lifecycle/C-ABI tests, device/simulator libraries and complete Expo simulator linking. |

API, Web and Pheme checks are component-level gates. Voice Integration exercises
their HTTP/streaming boundary, which package tests do not replace. iOS catches
Apple-target packaging and native app linkage, which Linux Rust tests cannot
cover. These scopes intentionally remain separate.

## iOS consolidation

The former Expo app and Pheme iOS FFI workflows are combined:

- `contracts` retains TypeScript, JavaScript/storage, fake-backed Swift runtime
  tests and the Expo JS bundle.
- `native-link` builds the production Whisper and reasoning XCFrameworks once
  through `npm run native:build`, which also prepares the Expo module inputs.
  It checks those inputs, runs the real Whisper Swift C-ABI smoke test and links
  the same libraries into the Expo simulator app.
- The `ios` matrix retains disabled and isolated-test device/simulator builds
  and Swift smoke tests. Their different capabilities and test-only symbols
  warrant separate checks.
- `scripts/test-build-ios.sh` runs only in CI / Pheme, instead of once there and
  again in each of the three former FFI matrix jobs.
- The old `feat/pheme-va-ios-ffi` push trigger is removed. Relevant pull requests
  and manual runs remain supported without running a second pipeline on pushes.

Production artifacts remain available as `PhemeVA-cpu-xcframework` and
`PhemeReasoning-cpu-xcframework`, alongside native build diagnostics. PR CI
does not download model weights. See the [native test guide](../ios-app/NATIVE_TESTING.md)
for physical-device and real-model acceptance requirements.

## Required checks and links

Existing job IDs and check names are retained where possible. The old
`iOS / whisper` matrix check is replaced by `native-link`, which now covers its
production packaging and Swift smoke test as well as complete app linkage.
If branch protection requires `iOS / whisper`, replace that requirement with
`native-link` after the new workflow has run. Repository protection settings
are not changed by edits to these files.

Update any external badges, workflow-file rules, or scripts using the old paths:

| Former filename | Current filename |
| --- | --- |
| `api-ci.yml` | `ci-api.yml` |
| `web-ci.yml` | `ci-web.yml` |
| `pheme-va-ci.yml` | `ci-pheme.yml` |
| `voice-stack-ci.yml` | `ci-voice-integration.yml` |
| `ios-app-ci.yml`, `pheme-va-ios-ci.yml` | `ci-ios.yml` |

Workflow-level path filters are unchanged in scope. An unrelated pull request
does not emit that workflow's checks; do not make path-filtered workflows
unconditionally required without accounting for skipped runs.

Validate workflow syntax and expressions with `actionlint` from the repository
root before changing triggers, jobs or action inputs.
