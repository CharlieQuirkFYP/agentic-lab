import type {
  BenchmarkReport, DurationGroup, EntityProjection, MetricGroup, ObservationFilters, ObservationProjection,
  LegacyPhoneObservation, PhoneMetricBatch, RecordReference, ReportOptions, StoredPhoneObservation,
} from './types.ts';
import { canonical, groupMetrics, ObservationConflictError, projectObservations, summarizeNumbers } from './projection.ts';
import { projectLegacyObservations } from './legacy.ts';

function freeze<T>(value: T): T {
  if (value !== null && typeof value === 'object') {
    for (const child of Object.values(value)) freeze(child);
    Object.freeze(value);
  }
  return value;
}
const entityKey = (installation: string, kind: string, id: string) => canonical([installation, kind, id]);

function selectedRecords(projection: ObservationProjection, filters: ObservationFilters): { records: StoredPhoneObservation[]; selected_record_keys: RecordReference[] } {
  const entities = [...projection.configurations, ...projection.conversations, ...projection.turns, ...projection.attempts, ...projection.sweeps, ...projection.jobs];
  const byID = new Map(entities.map(entity => [entityKey(entity.installation_id, entity.kind, entity.entity_id), entity]));
  const metricByRecord = new Map(projection.raw_metrics.flatMap(metric => metric.references.map(ref => [canonical([ref.installation_id, ref.record_id]), metric] as const)));
  const contextFields = { configuration_id: 'configuration', conversation_id: 'conversation', turn_id: 'turn', attempt_id: 'attempt', sweep_id: 'sweep', job_id: 'job' } as const;
  const selected = projection.records.filter(stored => {
    const { record, device } = stored;
    const entity = byID.get(entityKey(device.installation_id, record.kind, record.entity_id));
    const metric = metricByRecord.get(canonical([device.installation_id, record.record_id]));
    const context = entity?.context ?? metric?.context ?? record.context;
    const attempt = context.attempt_id ? byID.get(entityKey(device.installation_id, 'attempt', context.attempt_id)) : undefined;
    const conversation = context.conversation_id ? byID.get(entityKey(device.installation_id, 'conversation', context.conversation_id)) : undefined;
    const source = entity?.source ?? attempt?.source ?? conversation?.source ?? record.data.source;
    const stage = entity?.stage ?? metric?.stage ?? attempt?.stage ?? record.data.stage;
    const outcome = entity?.status ?? metric?.outcome ?? attempt?.status ?? record.data.status;
    return (!filters.installation_id || device.installation_id === filters.installation_id) &&
      (!filters.configuration_id || context.configuration_id === filters.configuration_id) &&
      (!filters.conversation_id || context.conversation_id === filters.conversation_id) &&
      (!filters.sweep_id || context.sweep_id === filters.sweep_id) &&
      (!filters.source || source === filters.source) && (!filters.stage || stage === filters.stage) && (!filters.outcome || outcome === filters.outcome) &&
      (filters.since_ms === undefined || record.timestamp_ms >= filters.since_ms) &&
      (filters.until_ms === undefined || record.timestamp_ms <= filters.until_ms);
  });
  const wanted = new Set(selected.map(stored => entityKey(stored.device.installation_id, stored.record.kind, stored.record.entity_id)));
  // Include every revision and explicit parent manifest of selected entities.
  for (const stored of selected) {
    const entity = byID.get(entityKey(stored.device.installation_id, stored.record.kind, stored.record.entity_id));
    const metric = metricByRecord.get(canonical([stored.device.installation_id, stored.record.record_id]));
    const context = entity?.context ?? metric?.context ?? stored.record.context;
    for (const [field, kind] of Object.entries(contextFields)) {
      const id = context[field as keyof typeof contextFields];
      if (id) wanted.add(entityKey(stored.device.installation_id, kind, id));
    }
  }
  return { records: projection.records.filter(stored => wanted.has(entityKey(stored.device.installation_id, stored.record.kind, stored.record.entity_id))),
    selected_record_keys: selected.map(stored => ({ installation_id: stored.device.installation_id, record_id: stored.record.record_id })) };
}

function durationGroups(projection: ObservationProjection, selectedKeys: readonly RecordReference[]): DurationGroup[] {
  const groups = new Map<string, { group: DurationGroup; values: number[]; eligible: number[] }>();
  const selected = new Set(selectedKeys.map(ref => canonical([ref.installation_id, ref.record_id])));
  for (const entity of [...projection.conversations, ...projection.turns, ...projection.attempts]) {
    if (entity.duration_ms === null || !selected.has(canonical([entity.installation_id, entity.record_id]))) continue;
    const identity = {
      installation_id: entity.installation_id, configuration_id: entity.context.configuration_id ?? null,
      entity_kind: entity.kind as 'conversation' | 'turn' | 'attempt', stage: entity.stage, source: entity.source, outcome: entity.status,
      hardware: entity.device.hardware, os_version: entity.device.os_version, app_version: entity.device.app_version, build: entity.device.build,
      dataset_revision: entity.dataset_revision, protocol_revision: entity.protocol_revision,
    };
    const id = canonical(identity);
    const entry = groups.get(id) ?? { group: { ...identity, numeric: summarizeNumbers([]), record_references: [] }, values: [], eligible: [] };
    entry.values.push(entity.duration_ms);
    if (entity.status === 'succeeded' && entity.completeness === 'complete') entry.eligible.push(entity.duration_ms);
    entry.group.record_references.push({ installation_id: entity.installation_id, record_id: entity.record_id });
    groups.set(id, entry);
  }
  return [...groups.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([, entry]) => ({ ...entry.group, numeric: summarizeNumbers(entry.values, entry.eligible) }));
}

function normalizeBatch(batch: PhoneMetricBatch): string {
  const { experiment_id: _experiment, events, ...rest } = batch;
  return canonical({ ...rest, events: events.map(event => { const { experiment_id: _eventExperiment, ...value } = event; return value; }) });
}

function excludeCanonicalCopies(projection: ObservationProjection, legacy: readonly LegacyPhoneObservation[]): { records: LegacyPhoneObservation[]; excluded: number } {
  const batches = new Map<string, Set<string>>();
  for (const stored of projection.records) {
    if (stored.record.kind !== 'metric_batch') continue;
    const batch = stored.record.data.batch as PhoneMetricBatch;
    const id = canonical([stored.device.installation_id, batch.run_id, batch.sequence_number]);
    const fingerprints = batches.get(id) ?? new Set<string>(); fingerprints.add(normalizeBatch(batch)); batches.set(id, fingerprints);
  }
  let excluded = 0;
  const records = legacy.filter(item => {
    if (!item.device) return true;
    const identity = canonical([item.device.installation_id, item.batch.run_id, item.batch.sequence_number]);
    const canonicalBatches = batches.get(identity);
    if (!canonicalBatches) return true;
    if (!canonicalBatches.has(normalizeBatch(item.batch))) throw new ObservationConflictError(item.device.installation_id, `canonical/legacy batch ${identity}`);
    excluded++; return false;
  });
  return { records, excluded };
}

/** Callers provide the clock and snapshot identity; returned exports cannot change. */
export function createBenchmarkReport(records: readonly StoredPhoneObservation[], options: ReportOptions): BenchmarkReport {
  if (!Number.isFinite(options.generated_at_ms) || !options.as_of) throw new TypeError('Report generation time and input watermark are required');
  const cloned = JSON.parse(JSON.stringify(records)) as StoredPhoneObservation[];
  const all = projectObservations(cloned, { filtered: options.input_already_filtered });
  const filters = { ...options.filters };
  const selected = selectedRecords(all, filters);
  const projection = options.input_already_filtered || !Object.keys(filters).length ? all : projectObservations(selected.records, { filtered: true });
  const selectedKeys = JSON.parse(JSON.stringify(options.selected_record_keys ?? selected.selected_record_keys)) as RecordReference[];
  const legacyInput = excludeCanonicalCopies(projection, JSON.parse(JSON.stringify(options.legacy_records ?? [])));
  const legacy = projectLegacyObservations(legacyInput.records, options.input_already_filtered ? {} : filters);
  legacy.canonical_duplicate_batches = legacyInput.excluded;
  const report: BenchmarkReport = {
    schema_version: 1, report_version: 2, generated_at_ms: options.generated_at_ms,
    input: { as_of: options.as_of, filters, record_count: projection.records.length, selected_record_keys: selectedKeys,
      filtered: options.input_already_filtered === true || Object.keys(filters).length > 0,
      record_references: projection.records.map(stored => ({ installation_id: stored.device.installation_id, record_id: stored.record.record_id })) },
    methodology: { p95_minimum_count: 20, percentile_method: 'nearest_rank', duration_unit: 'milliseconds',
      p95_eligibility: 'Duration distributions use one finite recorded duration_ms per selected latest entity revision, from explicitly succeeded executions with verified complete telemetry. At least 20 eligible entities are required per compatible group. Raw event and resource sample groups do not publish p95.',
      coverage: 'Selected retained records at the supplied input watermark; no lifetime-total claim. Execution status is independent of telemetry completeness. Resource means are sample means and maxima are sampled peaks. No power, energy, accuracy, or speech-to-audible-reply measurement is inferred.' },
    projection, duration_groups: durationGroups(projection, selectedKeys), legacy,
    metric_groups: groupMetrics([...projection.raw_metrics, ...legacy.raw_metrics]),
    totals: { canonical_records: projection.records.length, legacy_batches: legacy.batch_count,
      metric_batches: projection.counts.metric_batches + legacy.batch_count, metric_events: projection.counts.metric_events + legacy.event_count,
      conversations: projection.counts.conversations, turns: projection.counts.turns, attempts: projection.counts.attempts },
  };
  return freeze(report);
}

/** Raw observations are authoritative. Avoid serializing the expanded UI graph:
 * each conversation contains turns, attempts and repeated per-event model data.
 * That graph can exceed a phone JS runtime's maximum string length. */
export function reportToJSON(report: BenchmarkReport): string {
  const group = ({ configuration, event_references: _references, ...value }: MetricGroup) => ({ ...value,
    // Explicit configurations resolve through installation + configuration_id.
    // Legacy-only metadata has no such manifest and must remain inline.
    ...(value.configuration_id === null && configuration ? { configuration } : {}) });
  const entity = (item: EntityProjection) => {
    const { metric_groups, ...fields } = item;
    const value = { ...fields } as Record<string, unknown>;
    for (const child of ['turns', 'attempts'] as const) {
      if (Array.isArray(value[child])) {
        value[`${child.slice(0, -1)}_ids`] = (value[child] as EntityProjection[]).map(row => row.entity_id);
        delete value[child];
      }
    }
    return { ...value, metric_groups: metric_groups.map(group) };
  };
  const p = report.projection;
  const { raw_metrics: _legacyMetrics, metric_groups: _legacyGroups, ...legacy } = report.legacy;
  return JSON.stringify({ ...report,
    encoding: 'canonical_records_with_flat_summaries_v2',
    reconstruction: 'Rebuild expanded views and metric event memberships with createBenchmarkReport(projection.records, { generated_at_ms, as_of: input.as_of, filters: input.filters, input_already_filtered: input.filtered, selected_record_keys: input.selected_record_keys, legacy_records: legacy.records }). Configuration references use installation_id and configuration_id; child IDs refer to the corresponding flat entity array.',
    projection: { schema_version: p.schema_version, records: p.records,
      configurations: p.configurations.map(entity), conversations: p.conversations.map(entity),
      turns: p.turns.map(entity), attempts: p.attempts.map(entity), sweeps: p.sweeps.map(entity), jobs: p.jobs.map(entity),
      metric_groups: p.metric_groups.map(group), unresolved_references: p.unresolved_references,
      counts: p.counts, coverage: p.coverage },
    legacy, metric_groups: report.metric_groups.map(group),
  });
}

/** Neutralize spreadsheet formulas while preserving numeric zero and null separately. */
function csv(value: unknown): string {
  if (value === null || value === undefined) return '';
  let content = typeof value === 'object' ? canonical(value) : String(value);
  if (typeof value === 'string' && /^[\s\u0000-\u001f]*[=+@-]/.test(content)) content = `'${content}`;
  return `"${content.replace(/"/g, '""')}"`;
}
const csvColumns = ['row_type', 'as_of', 'generated_at_ms', 'installation_id', 'configuration_id', 'conversation_id', 'turn_id', 'attempt_id', 'sweep_id', 'job_id', 'record_id', 'revision', 'stage', 'outcome', 'completeness', 'name', 'unit', 'scope', 'source', 'value', 'value_type', 'unavailable_reason', 'count', 'mean', 'median', 'min', 'max', 'p95', 'p95_eligible_count', 'raw_references', 'data'] as const;

export function reportToCSV(report: BenchmarkReport): string {
  const rows: Record<string, unknown>[] = [{ row_type: 'report', data: { filters: report.input.filters, methodology: report.methodology, totals: report.totals, coverage: report.projection.coverage } }];
  const entities: EntityProjection[] = [...report.projection.configurations, ...report.projection.conversations, ...report.projection.turns, ...report.projection.attempts, ...report.projection.sweeps, ...report.projection.jobs];
  for (const entity of entities) rows.push({ row_type: entity.kind, installation_id: entity.installation_id, ...entity.context,
    record_id: entity.record_id, revision: entity.revision, stage: entity.stage, outcome: entity.status, completeness: entity.completeness,
    value: entity.duration_ms, value_type: entity.duration_ms === null ? 'null' : 'number', unit: 'milliseconds', raw_references: entity.record_references, data: { device: entity.device, ...entity.data } });
  for (const group of report.metric_groups) rows.push({ row_type: 'metric_summary', ...group, ...group.numeric,
    count: group.numeric.count, raw_references: group.event_references, data: { status_counts: group.status_counts, unavailable_count: group.unavailable_count,
      unavailable_reasons: group.unavailable_reasons, hardware: group.hardware, build: group.build, app_version: group.app_version, os_version: group.os_version,
      configuration: group.configuration, dataset_revision: group.dataset_revision, protocol_revision: group.protocol_revision } });
  for (const group of report.duration_groups) rows.push({ row_type: 'duration_summary', ...group, ...group.numeric, unit: 'milliseconds',
    name: `${group.entity_kind}_duration`, raw_references: group.record_references, data: group });
  for (const metric of [...report.projection.raw_metrics, ...report.legacy.raw_metrics]) rows.push({ row_type: 'metric_event', installation_id: metric.installation_id,
    ...metric.context, ...metric.event, record_id: metric.references[0]?.record_id, stage: metric.stage, outcome: metric.outcome, completeness: metric.completeness,
    value_type: metric.event.value === null ? 'null' : typeof metric.event.value, raw_references: metric.references, data: { device: metric.device, run_id: metric.event.run_id, sequence: metric.event.sequence, timestamp_ms: metric.event.timestamp_ms } });
  for (const stored of report.projection.records) rows.push({ row_type: 'raw_record', installation_id: stored.device.installation_id,
    ...stored.record.context, record_id: stored.record.record_id, revision: stored.record.revision, data: stored });
  for (const stored of report.legacy.records) rows.push({ row_type: 'legacy_batch', installation_id: stored.device?.installation_id ?? null, data: stored });
  return [csvColumns.map(csv).join(','), ...rows.map(row => csvColumns.map(column => csv(column === 'as_of' ? report.input.as_of : column === 'generated_at_ms' ? report.generated_at_ms : row[column])).join(','))].join('\r\n') + '\r\n';
}

const md = (value: unknown): string => value === null || value === undefined ? 'unavailable' : String(value).replace(/[\\`*_{}[\]<>]/g, '\\$&').replace(/\|/g, '\\|').replace(/[\r\n]+/g, ' ');
const show = (value: number | null) => value === null ? 'unavailable' : String(Math.round(value * 1000) / 1000);
const groupLabel = (group: MetricGroup) => `${group.installation_id} / ${group.configuration_id ?? (group.configuration ? canonical(group.configuration) : 'unknown configuration')} / ${group.stage ?? 'unknown stage'} / ${group.outcome ?? 'unknown outcome'} / ${group.build} / ${group.dataset_revision ?? 'unknown dataset'} / ${group.protocol_revision ?? 'unknown protocol'}`;

export function reportToMarkdown(report: BenchmarkReport): string {
  const p = report.projection;
  const lines = [
    '# Phone benchmark report', '', `Report format revision: ${report.report_version}. Generated at Unix milliseconds: ${report.generated_at_ms}.`,
    `Input watermark: ${md(report.input.as_of)}. Filters: ${md(canonical(report.input.filters))}.`, '', report.methodology.coverage, '',
    `Retained counts: ${p.counts.installations} installations, ${report.totals.conversations} explicit conversations, ${report.totals.turns} turns, ${report.totals.attempts} attempts, ${p.counts.jobs} planned/job entities, ${report.totals.metric_batches} metric batches, ${report.totals.metric_events} metric events.`,
    `Telemetry: ${p.coverage.completeness}; ${p.coverage.unresolved_reference_count} unresolved references, ${p.coverage.sequence_gaps.length} observed sequence gaps, ${p.coverage.dropped_events} dropped events. Legacy batches: ${report.legacy.batch_count}; legacy conversation count is unknown.`, '',
    '## Models and configuration', '', '| Installation | Configuration | ASR model / SHA-256 | Reasoning model / SHA-256 | Settings / runtime |', '| --- | --- | --- | --- | --- |',
    ...p.configurations.map(c => `| ${md(c.installation_id)} | ${md(c.entity_id)} | ${md(c.asr?.name)} / ${md(c.asr?.sha256)} | ${md(c.reasoning?.name)} / ${md(c.reasoning?.sha256)} | ${md(canonical({ settings: c.settings, runtime: c.runtime, role_sha256: c.role_sha256, device: c.device }))} |`), '',
    '## Observed outcomes', '', '| Entity | Outcome | Count |', '| --- | --- | --- |',
    ...Object.entries(p.counts.outcomes).flatMap(([kind, outcomes]) => Object.entries(outcomes).map(([status, count]) => `| ${kind} | ${md(status)} | ${count} |`)), '',
    '## Conversations', '', '| Installation / conversation | Source / case / repetition | Status | Telemetry | Turns / attempts | Recorded duration (ms) | Raw revision |', '| --- | --- | --- | --- | --- | --- | --- |',
    ...p.conversations.map(c => `| ${md(c.installation_id)} / ${md(c.entity_id)} | ${md(c.source)} / ${md(c.data.case_id)} / ${md(c.data.repetition)} | ${md(c.status)} | ${c.completeness} | ${c.turns.length} / ${c.attempts.length} | ${show(c.duration_ms)} | ${md(c.record_id)} |`), '',
    '## Metrics by conversation', '', '| Installation / conversation | Metric / stage / unit / scope / source / outcome | Numeric count | Mean | Min | Max | Status / unavailable | Raw events |', '| --- | --- | --- | --- | --- | --- | --- | --- |',
    ...p.conversations.flatMap(c => c.metric_groups.map(g => `| ${md(c.installation_id)} / ${md(c.entity_id)} | ${md(g.name)} / ${md(g.stage)} / ${md(g.unit)} / ${md(g.scope)} / ${md(g.source)} / ${md(g.outcome)} | ${g.numeric.count} | ${show(g.numeric.mean)} | ${show(g.numeric.min)} | ${show(g.numeric.max)} | ${md(canonical(g.status_counts))} / ${g.unavailable_count}: ${md(canonical(g.unavailable_reasons))} | ${md(canonical(g.event_references))} |`)), '',
    '## Duration distributions', '', report.methodology.p95_eligibility, '',
    '| Installation / configuration | Entity / stage / outcome | Count | Mean | Median | Min | Max | p95 | Eligible count | Raw revisions |', '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |',
    ...report.duration_groups.map(g => `| ${md(g.installation_id)} / ${md(g.configuration_id)} | ${g.entity_kind} / ${md(g.stage)} / ${md(g.outcome)} | ${g.numeric.count} | ${show(g.numeric.mean)} | ${show(g.numeric.median)} | ${show(g.numeric.min)} | ${show(g.numeric.max)} | ${g.numeric.p95 === null ? 'insufficient eligible samples' : show(g.numeric.p95)} | ${g.numeric.p95_eligible_count} | ${md(canonical(g.record_references))} |`), '',
    '## All observed metric families', '', '| Compatible group | Metric / unit / scope / source | Numeric count | Mean | Median | Min | Max | p95 | Status counts | Unavailable | Raw events |', '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |',
    ...report.metric_groups.map(g => `| ${md(groupLabel(g))} | ${md(g.name)} / ${md(g.unit)} / ${md(g.scope)} / ${md(g.source)} | ${g.numeric.count} | ${show(g.numeric.mean)} | ${show(g.numeric.median)} | ${show(g.numeric.min)} | ${show(g.numeric.max)} | ${g.numeric.p95 === null ? 'insufficient eligible samples' : show(g.numeric.p95)} | ${md(canonical(g.status_counts))} | ${g.unavailable_count}: ${md(canonical(g.unavailable_reasons))} | ${md(canonical(g.event_references))} |`), '',
    '## Coverage and raw input references', '', md(canonical(p.coverage)), '', md(canonical(p.unresolved_references)), '',
    ...report.input.record_references.map(ref => `- ${md(ref.installation_id)} / ${md(ref.record_id)}`), '', report.legacy.note, '',
  ];
  return lines.join('\n');
}
