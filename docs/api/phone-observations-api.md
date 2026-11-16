# Phone observation API

Additive endpoints under `/api/v1/phone`; existing `/metrics` and experiments remain compatible. Uploads store phone observations only and never launch inference.

`POST /observations` accepts the frozen [version 1 envelope](../plans/phone-benchmark-implementation-contract.md), at most 500 records / 4 MiB. Returns HTTP 202 `{status:"accepted",record_count:2,accepted_record_ids:["..."]}`. Each chunk is atomic. Exact retries succeed; conflicting record ID, producer stream/sequence, or entity kind/ID/revision within one installation return 409. Invalid input returns 400.

Read filters: `installation_id`, `conversation_id`, `sweep_id`, `configuration_id`, `since_ms`, `until_ms`, `source` (`manual|sweep|acceptance`), `stage` (`transcription|reasoning|loading|warmup`), `outcome`. Date bounds are inclusive producer record timestamps. Attribution filters resolve explicit parent references at the snapshot, including late-arriving parents. Unknown legacy attribution is excluded by filters requiring that attribution. `as_of` is an opaque snapshot string, not a date; omit for a fresh snapshot. Repeat it on every request contributing to one report.

`GET /observations?limit=100&cursor=...&as_of=...` returns `{schema_version:1,as_of,records:[{device,record,received_at_ms}],total,next_cursor,selected_record_keys:[{installation_id,record_id}],filtered}`. Limit 1–100, default 100. `next_cursor` is null at completion. Cursor binds the snapshot and filters; changing filters or supplying a different snapshot returns 400. Configuration manifests and lifecycle ancestors referenced by selected observations are included so clients can reproduce attribution and outcomes. All immutable revisions of selected entities are included. `selected_record_keys` identifies direct filter matches on that page; other returned records are supporting ancestors/revisions. Retain supporting entities in relationship counts, but do not treat their durations as selected observations. Pass the accumulated keys and `filtered` to shared reporting so filtering does not create false stream gaps.

`GET /legacy-observations` has the same snapshot, cursor and applicable filters, plus an optional exact `run_id`, and returns `{schema_version:1,as_of,batches:[{device,batch,received_at_ms}],total,next_cursor}`. Device can be null for unattributed historical records. These are legacy v1 batches, without invented conversation identities. An exact batch represented in canonical observations is excluded. This endpoint provides full historical snapshot exports without offset races. Historical `received_at_ms` is the original experiment registration time because the old table did not retain individual batch receipt times; it is not the actual receipt time of every later batch.

`GET /overview` accepts the same filters and optional `as_of`, with no pagination. All counts and series cover the entire selected retained dataset. Only recent activity arrays are bounded to 20. Response TypeScript shape:

```typescript
interface PhoneOverview {
  schema_version: 1;
  as_of: string;
  source: 'device_reported';
  coverage: {
    record_count: number; metric_batch_count: number; event_count: number;
    legacy_batch_count: number; legacy_conversation_count: null;
    first_timestamp_ms: number | null; last_timestamp_ms: number | null;
    last_received_at_ms: number | null;
    unresolved_record_count: number;
    finalization: { complete: number; partial: number; unknown: number; dropped_events: number };
    retained_history: true;
  };
  counts: {
    configurations: number;
    conversations: { total: number; outcomes: Record<string, number> };
    turns: { total: number; outcomes: Record<string, number> };
    attempts: { total: number; outcomes: Record<string, number> };
    sweeps: { total: number; outcomes: Record<string, number> };
    jobs: { total: number; outcomes: Record<string, number> };
  };
  series: Array<PhoneMetricSeries>;
  current_conversation: PhoneActivity | null;
  recent_conversations: PhoneActivity[];
  recent_runs: PhoneActivity[];
  recent_sweeps: PhoneActivity[];
}
interface PhoneMetricSeries {
  installation_id: string | null; device: Device | null;
  configuration_id: string | null; configuration: Record<string, unknown> | null;
  stage: string | null; dataset_revision: string | null; protocol_revision: string | null;
  outcome: string | null;
  name: string; unit: string; scope: string; source: string;
  observation_count: number; numeric_count: number; unavailable_count: number;
  unavailable_reasons: string[]; min: number | null; mean: number | null; max: number | null;
  latest: number | string | boolean | null; latest_timestamp_ms: number;
  status_counts: Record<string, number>;
}
interface PhoneActivity {
  installation_id: string | null; device: Device | null;
  entity_id: string; kind: string; status: string | null;
  timestamp_ms: number; started_at_ms: number | null; finished_at_ms: number | null;
  configuration_id: string | null;
  configuration: Record<string, unknown> | null; // includes actual asr/reasoning names + hashes
  context: Record<string, string>;
  source: string | null; stage: string | null;
  metric_batch_count: number; event_count: number;
  completeness: 'complete' | 'partial' | 'unknown';
  metrics: PhoneMetricSeries[];
}
```

`current_conversation` is the most recently observed active/running explicit conversation; if none is active it is the latest observed conversation. Recent conversations also include it. Runs are explicit attempts plus unmatched legacy native runs. Legacy statuses and configurations remain null unless actually recorded. Series separate installation, device/build, configuration, stage, dataset/protocol, outcome and original metric name/unit/scope/source. Numeric summaries describe reported samples; status values remain categorical, valid zeroes remain zero, and absent sensors remain unavailable. No percentile is computed in Go; canonical shared reporting owns eligible-duration statistics.

Metric uploads may retain unbound session/idle samples without an attempt; their attribution remains unknown. New inference producers register attempts before invocation. Canonical batch identity includes the installation and attempt (or runtime session for unbound samples), so restarting native counters does not merge independent executions. Conflicting reuse of that batch identity also returns 409.

A complete finalization must include `target_revision`, unique `required_record_ids`, matching `record_count`, `event_count` and zero `dropped_events`. The read projection verifies that the target revision and every referenced record exist in the snapshot, that every contextual record is covered, that deduplicated event counts match, and that references and descendant attempt/conversation finalizations are complete. Missing evidence or later target revisions downgrade completeness to partial. An absent finalization remains unknown. A terminal execution outcome by itself never establishes telemetry completeness.

The overview uses a consistent SQLite read transaction and aggregates the entire retained selected snapshot in memory. Receipt cursors remain stable under later appends. The activity arrays are capped at 20; summary series are not truncated. Large archives may eventually require indexed incremental aggregation; the current implementation does not maintain a second inference state machine.

Example active conversation activity (the same object shape appears in `current_conversation` and `recent_conversations`):

```json
{
  "installation_id": "installation-example",
  "device": {"installation_id":"installation-example","hardware":"iPhone","os_version":"26","app_version":"1","build":"42"},
  "entity_id": "conversation-example",
  "kind": "conversation",
  "status": "active",
  "timestamp_ms": 1794300000100,
  "started_at_ms": 1794300000000,
  "finished_at_ms": null,
  "configuration_id": "configuration-example",
  "configuration": {
    "asr": {"id":"whisper-tiny","name":"Whisper tiny","sha256":"artifact-hash-from-manifest"},
    "reasoning": {"id":"qwen","name":"Qwen 0.5B","sha256":"artifact-hash-from-manifest"},
    "settings": {},
    "runtime": {}
  },
  "context": {"configuration_id":"configuration-example","conversation_id":"conversation-example"},
  "source": "manual",
  "stage": null,
  "metric_batch_count": 0,
  "event_count": 0,
  "completeness": "unknown",
  "metrics": []
}
```

This illustrative object describes registration before samples arrive; it is not a device measurement result.
