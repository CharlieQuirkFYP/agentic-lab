# Shared phone benchmark reports

This pure TypeScript module consumes the version 1 observation wire contract from
`docs/plans/phone-benchmark-implementation-contract.md`. It has no browser, Node,
React Native, network, clock, or filesystem dependencies. Platform callers supply
all records, the snapshot watermark, and the report generation time.

```typescript
import { createBenchmarkReport, reportToJSON } from './index.ts';

const report = createBenchmarkReport(records, {
  generated_at_ms: generationTime,
  as_of: snapshotWatermark,
  filters: { sweep_id: sweepID },
});
const contents = reportToJSON(report);
```

`projectObservations(records)` returns the latest revision of each explicit
configuration/conversation/turn/attempt/job/sweep, their relationships, metric
groups, raw event references, observed outcome counts and coverage. Exact retries
deduplicate; conflicting record, stream/sequence, entity revision or native event
identities throw `ObservationConflictError`. Installation scope is always part of
identity. Missing parent references remain unresolved; no terminal outcome or
conversation boundary is inferred from absence.

Server export adapters must read every cursor page from both canonical and legacy
endpoints using the same `as_of`. Pass `input_already_filtered: true`, the exact
filters, all per-page `selected_record_keys`, and optional `legacy_records` to
`createBenchmarkReport`. Selected keys distinguish matching measurements from
supporting ancestor records. Ancestors count as contributing conversations/turns,
but support-only durations do not enter distributions. A filtered snapshot does
not infer loss from intentionally omitted global stream positions.

Complete finalization requires a matching `target_revision`, unique
`required_record_ids` covering every retained non-finalization descendant and the
target itself, exact `record_count`/`event_count`, no drops, resolved relationships,
and complete descendant attempts. A bare complete marker remains partial. Status
and completeness are independent. Missing stream prefixes/suffixes cannot be
detected from sequence gaps alone; explicit finalization references supply proof.

Numeric groups preserve configuration/model names and hashes, installation,
hardware/OS/app/build, dataset/protocol revision, stage, execution outcome and each
metric's original name/unit/scope/source. Null, zero, string/boolean status and
unavailability reasons stay distinct. Summary means use individual finite samples;
resource means are sample means and maxima are sampled peaks. Raw event groups
never publish p95. Duration groups use one recorded duration per selected latest
entity, and nearest-rank p95 requires at least 20 successful, complete entities in
that compatible group. No timing boundary is invented from start/finish times.

Legacy records retain unknown conversations, unknown completeness and isolated
unattributed containers. Only explicit installation-scoped native-run/turn links
can associate legacy metadata. Canonical/legacy copies normalize server-assigned
experiment IDs before deduplication; conflicting cross-source identities reject
the report. Distinct canonical attempts remain distinct even when native counters
restart.

`createBenchmarkReport` clones and recursively freezes its input-derived result.
JSON includes manifests, relationships, metrics, references and raw records. CSV
uses typed rows for reports/entities/summaries/raw events/raw records, preserves
null separately from zero via `value_type`, and escapes spreadsheet formulas.
Markdown includes overall, configuration, conversation, duration, metric and
coverage sections with raw references. Later uploads require a new report.

Run deterministic fixture tests (these are **not phone measurements**):

```sh
node --test shared/phone-benchmarks/tests/*.test.ts
```

JSON report format revision 2 stores canonical input records once and serializes
flat entity summaries. Conversation/turn children use `turn_ids` and
`attempt_ids`; expanded `raw_metrics` arrays are omitted because they repeat
model/device metadata for every event. Repeated per-summary event-reference lists
are reconstructed from those same raw records. Explicit metric configurations resolve
through `(installation_id, configuration_id)` and the configuration manifests;
legacy inline metadata remains when no manifest exists. JSON includes an
`encoding` and reconstruction recipe. Re-run `createBenchmarkReport` with its
raw records, original filters, selected/supporting keys, watermark and legacy
records to reproduce the full in-memory hierarchy and all statistics. This
avoids the phone JavaScript string limit reached by larger version-1 exports.
Version-1 reports remain readable as expanded snapshots. CSV and Markdown keep
their attributed event/conversation tables and report the new format revision.
