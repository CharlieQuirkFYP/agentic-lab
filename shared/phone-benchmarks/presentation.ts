import { canonical, summarizeNumbers } from './projection.ts';
import type {
  AttemptProjection, Completeness, EntityProjection, EventReference, MetricValue, NumericSummary,
  ObservationProjection, PhoneDevice, PhoneObservationContext, ProjectedMetric, RecordReference, TurnProjection,
} from './types.ts';

/** Presentation derivations never alter the canonical records or recorded-duration distributions. */
export interface ResultsScope {
  installation_id?: string; configuration_id?: string; conversation_id?: string;
  turn_id?: string; attempt_id?: string; sweep_id?: string;
}
export interface ProcessingSegment {
  attempt_id: string; stage: string; status: string | null; duration_ms: number | null;
  completeness: Completeness; started_at_ms: number | null; finished_at_ms: number | null;
  record_references: RecordReference[];
}
export interface ProcessingTotal {
  duration_ms: number | null; partial: boolean; attempt_ids: string[]; missing_stages: string[];
  reasons: string[]; definition: string;
}
export interface ProcessingTurn {
  turn_id: string; conversation_id: string | null; installation_id: string; turn_index: number | null;
  status: string | null; segments: ProcessingSegment[]; recordedProcessing: ProcessingTotal;
}
export interface MetricPoint {
  timestamp_ms: number; elapsed_ms: number; value: MetricValue; displayValue: MetricValue;
  breakBefore: boolean; segment: number; context: PhoneObservationContext; stage: string | null;
  outcome: string | null; references: EventReference[]; unavailable_reason?: string;
}
export interface SampleStatistics { count: number; latest: number | null; min: number | null; max: number | null; mean: number | null }
export interface ResultsMetric {
  key: string; name: string; label: string; unit: string; displayUnit: string; scope: string; source: string;
  installation_id: string; configuration_id: string | null; device: PhoneDevice;
  configuration: Record<string, unknown> | null; dataset_revision: string | null; protocol_revision: string | null;
  kind: 'numeric' | 'categorical' | 'mixed' | 'unavailable'; points: MetricPoint[]; rawPoints: MetricPoint[];
  stats: SampleStatistics; rawStats: SampleStatistics; sampleCount: number; unavailableCount: number;
  operationCount: number; description: string; statusCounts: Array<{ value: string | boolean; count: number }>;
  downsampled: boolean; omittedSegments: number;
}
export interface ResultsPresentation {
  scope: ResultsScope; turns: ProcessingTurn[]; recordedProcessing: ProcessingTotal;
  outcomes: Record<string, number>; lifecycleStatuses: Array<{ conversation_id: string; status: string | null }>;
  interval: { start_ms: number | null; end_ms: number | null }; metrics: ResultsMetric[];
  memory: ResultsMetric[]; thermal: ResultsMetric[]; capabilities: Array<{ name: string; label: string; reasons: string[] }>;
  coverage: string[]; typicalProcessingMs: number | null;
}

const terminal = new Set(['succeeded', 'failed', 'cancelled', 'interrupted', 'skipped']);
const processingStages = new Set(['transcription', 'reasoning']);
const resourceNames = new Set(['resident_memory_bytes', 'physical_footprint_bytes', 'cpu_percent', 'thermal_state',
  'battery_level', 'battery_state', 'screen_brightness', 'low_power_mode', 'gpu_utilization', 'power', 'energy',
  'sample_monotonic_ms', 'resource_stage', 'sample_boundary', 'sample_interval_ms']);
const optionalNames = new Set(['gpu_utilization', 'power', 'energy']);
const PROCESSING_DEFINITION = 'Sum of distinct recorded transcription and reasoning native-call durations, including retries. Excludes loading, warmup, review, playback and idle time.';
const labels: Record<string, string> = {
  transcription: 'Speech processing', reasoning: 'Reply generation', loading: 'Model loading', warmup: 'Warmup',
  resident_memory_bytes: 'App memory', physical_footprint_bytes: 'App physical footprint', cpu_percent: 'App CPU',
  thermal_state: 'Thermal state', battery_level: 'Battery level', battery_state: 'Charging state',
  low_power_mode: 'Low power mode', screen_brightness: 'Screen brightness', gpu_utilization: 'GPU utilization',
  power: 'Power', energy: 'Energy',
};
export function resultMetricLabel(name: string): string { return labels[name] ?? name.replaceAll('_', ' '); }
const object = (value: unknown): Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
const string = (value: unknown): string | null => typeof value === 'string' ? value : null;
const finite = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value);
const duration = (value: unknown): number | null => finite(value) && value >= 0 ? value : null;
const identity = (installation: string, id: string) => canonical([installation, id]);
const unique = <T>(values: T[]): T[] => [...new Set(values)];
function inScope(item: { installation_id: string; context: PhoneObservationContext }, scope: ResultsScope): boolean {
  return (!scope.installation_id || item.installation_id === scope.installation_id) &&
    Object.entries(scope).every(([key, value]) => key === 'installation_id' || !value || item.context[key as keyof PhoneObservationContext] === value);
}
function counts(items: Array<{ status: string | null }>): Record<string, number> {
  const result: Record<string, number> = {};
  for (const item of items) result[item.status ?? 'unknown'] = (result[item.status ?? 'unknown'] ?? 0) + 1;
  return result;
}
function total(segments: ProcessingSegment[], missing: string[] = [], extraReasons: string[] = []): ProcessingTotal {
  const values = segments.flatMap(segment => segment.duration_ms === null ? [] : [segment.duration_ms]);
  const reasons = unique([...extraReasons, ...missing.map(stage => `Missing ${resultMetricLabel(stage).toLowerCase()} attempt.`),
    ...segments.flatMap(segment => [
      ...(segment.duration_ms === null ? [`No recorded duration for ${segment.attempt_id}.`] : []),
      ...(!terminal.has(segment.status ?? '') ? [`${segment.attempt_id} has not finished.`] : []),
      ...(segment.completeness !== 'complete' ? [`Collection is ${segment.completeness} for ${segment.attempt_id}.`] : []),
    ])]);
  return { duration_ms: values.length ? values.reduce((a, b) => a + b, 0) : null,
    partial: reasons.length > 0 || !segments.length, attempt_ids: segments.map(segment => segment.attempt_id),
    missing_stages: unique(missing), reasons, definition: PROCESSING_DEFINITION };
}
function processingTurn(turn: TurnProjection, attempts: AttemptProjection[], projection: ObservationProjection, narrowed: boolean): ProcessingTurn {
  const segments = attempts.filter(attempt => processingStages.has(attempt.stage ?? '')).map(attempt => ({
    attempt_id: attempt.entity_id, stage: attempt.stage!, status: attempt.status, duration_ms: duration(attempt.duration_ms),
    completeness: attempt.completeness, started_at_ms: attempt.started_at_ms, finished_at_ms: attempt.finished_at_ms,
    record_references: attempt.record_references,
  })).sort((a, b) => (a.started_at_ms ?? Infinity) - (b.started_at_ms ?? Infinity) || a.attempt_id.localeCompare(b.attempt_id));
  const sweep = projection.sweeps.find(item => item.installation_id === turn.installation_id && item.entity_id === turn.context.sweep_id);
  const mode = object(sweep?.data.protocol).mode ?? turn.data.mode;
  const input = turn.data.input_kind;
  const expected = narrowed ? [] : input === 'text' ? ['reasoning'] : mode === 'asr_only' ? ['transcription'] :
    input === 'audio' || input === 'fixed_wav' ? ['transcription', 'reasoning'] : [];
  const reasons = [
    ...(!terminal.has(turn.status ?? '') && !narrowed ? ['Turn has not finished.'] : []),
    ...(!expected.length && !narrowed ? ['Expected processing stages are not recorded.'] : []),
  ];
  return { turn_id: turn.entity_id, conversation_id: turn.context.conversation_id ?? null,
    installation_id: turn.installation_id, turn_index: turn.turn_index, status: turn.status, segments,
    recordedProcessing: total(segments, expected.filter(stage => !segments.some(segment => segment.stage === stage)), reasons) };
}

function sampleStatistics(points: readonly MetricPoint[], display: boolean): SampleStatistics {
  const values = points.flatMap(point => { const value = display ? point.displayValue : point.value; return finite(value) ? [value] : []; });
  let min: number | null = null, max: number | null = null, sum = 0;
  for (const value of values) { min = min === null ? value : Math.min(min, value); max = max === null ? value : Math.max(max, value); sum += value; }
  return { count: values.length, latest: values.at(-1) ?? null, min, max, mean: values.length ? sum / values.length : null };
}

/** A bounded display only. Segment IDs survive omissions, so disjoint executions never join. */
export function downsampleMetricPoints(points: readonly MetricPoint[], budget = 200): MetricPoint[] {
  const limit = Math.max(4, Math.min(200, Math.floor(budget) || 200));
  if (points.length <= limit) return [...points];
  const selected = new Set<number>([0, points.length - 1]);
  const boundaries: number[] = [];
  let minIndex = -1, maxIndex = -1;
  points.forEach((point, index) => {
    if (finite(point.displayValue)) {
      if (minIndex < 0 || point.displayValue < (points[minIndex].displayValue as number)) minIndex = index;
      if (maxIndex < 0 || point.displayValue > (points[maxIndex].displayValue as number)) maxIndex = index;
    }
    if (index && (point.segment !== points[index - 1].segment ||
      !finite(point.displayValue) && point.displayValue !== points[index - 1].displayValue)) boundaries.push(index - 1, index);
  });
  if (minIndex >= 0) selected.add(minIndex);
  if (maxIndex >= 0) selected.add(maxIndex);
  const uniqueBoundaries = unique(boundaries);
  // If boundaries exceed the budget, keep time coverage and extrema; expose
  // omittedSegments and retain every original boundary in rawPoints.
  if (uniqueBoundaries.length + selected.size <= limit) uniqueBoundaries.forEach(index => selected.add(index));
  const bucketCount = Math.max(1, Math.floor((limit - selected.size) / 2));
  for (let bucket = 0; bucket < bucketCount && selected.size < limit; bucket++) {
    const start = Math.floor(bucket * points.length / bucketCount), end = Math.floor((bucket + 1) * points.length / bucketCount);
    let low = -1, high = -1;
    for (let index = start; index < end; index++) if (finite(points[index].displayValue)) {
      if (low < 0 || (points[index].displayValue as number) < (points[low].displayValue as number)) low = index;
      if (high < 0 || (points[index].displayValue as number) > (points[high].displayValue as number)) high = index;
    }
    for (const index of low < 0 ? [start, end - 1] : [low, high]) if (selected.size < limit) selected.add(index);
  }
  // Flat series still show their time range when extrema indices coincide.
  for (let step = 1; step < limit && selected.size < limit; step++) selected.add(Math.floor(step * (points.length - 1) / (limit - 1)));
  let previous: number | undefined;
  return [...selected].sort((a, b) => a - b).map(index => {
    const point = points[index];
    const categoricalGap = previous !== undefined && index > previous + 1 && !finite(point.displayValue);
    const result = { ...point, breakBefore: previous === undefined || point.segment !== points[previous].segment || point.breakBefore || categoricalGap };
    previous = index;
    return result;
  });
}

function metricDescription(name: string, unit: string, scope: string): string {
  if (name === 'resident_memory_bytes' || name === 'physical_footprint_bytes') return `${scope === 'process' ? 'Whole-app process' : 'Recorded'} memory${unit === 'bytes' ? ' in MiB (1,048,576 bytes)' : ''}. Maximum is the highest sampled reading; the average is over samples.`;
  if (name === 'cpu_percent') return unit === 'percent_one_core'
    ? 'Whole-app process CPU. 100% represents one fully used core; values may exceed 100%. Average is over samples.'
    : 'Recorded CPU readings. Consult the source, scope and unit for their measurement basis. Average is over samples.';
  if (name === 'thermal_state') return 'Recorded device thermal categories, not a temperature. Gaps have no recorded state.';
  if (name === 'battery_level') return 'Recorded device battery percentage. Charging context is separate; these readings do not measure watts, joules or endurance.';
  return 'Recorded readings with original source, scope and unit. Sample statistics are not execution percentiles or time-weighted averages.';
}

function metricSeries(metrics: ProjectedMetric[], start: number): ResultsMetric[] {
  const groups = new Map<string, ProjectedMetric[]>();
  for (const metric of metrics) {
    const key = canonical([metric.installation_id, metric.device, metric.context.configuration_id ?? null, metric.configuration,
      metric.dataset_revision, metric.protocol_revision, metric.event.name, metric.event.unit, metric.event.scope, metric.event.source,
      resourceNames.has(metric.event.name) ? null : metric.stage]);
    const group = groups.get(key) ?? []; group.push(metric); groups.set(key, group);
  }
  return [...groups].map(([key, group]) => {
    group.sort((a, b) => a.event.timestamp_ms - b.event.timestamp_ms || a.event.sequence - b.event.sequence || canonical(a.references).localeCompare(canonical(b.references)));
    const first = group[0], { event } = first;
    const divisor = event.unit === 'bytes' ? 1024 * 1024 : 1;
    const displayUnit = event.unit === 'bytes' ? 'MiB' : event.unit === 'percent_one_core' || event.unit === 'percent' ? '%' : event.unit;
    let segment = 0;
    const rawPoints: MetricPoint[] = group.map((metric, index) => {
      const previous = group[index - 1];
      const operation = (item: ProjectedMetric) => canonical([item.context.conversation_id, item.context.turn_id, item.context.attempt_id, item.context.runtime_session_id, item.event.run_id]);
      const gap = !previous || operation(previous) !== operation(metric) || metric.event.timestamp_ms - previous.event.timestamp_ms > 2000 ||
        previous.event.value === null || metric.event.value === null;
      if (gap) segment++;
      return { timestamp_ms: metric.event.timestamp_ms, elapsed_ms: metric.event.timestamp_ms - start, value: metric.event.value,
        displayValue: finite(metric.event.value) ? metric.event.value / divisor : metric.event.value,
        breakBefore: gap, segment, context: metric.context, stage: metric.stage, outcome: metric.outcome,
        references: metric.references, ...(metric.event.unavailable_reason ? { unavailable_reason: metric.event.unavailable_reason } : {}) };
    });
    const points = downsampleMetricPoints(rawPoints), stats = sampleStatistics(rawPoints, true), rawStats = sampleStatistics(rawPoints, false);
    const statusCounts: ResultsMetric['statusCounts'] = [];
    for (const point of rawPoints) if (typeof point.value === 'string' || typeof point.value === 'boolean') {
      const existing = statusCounts.find(item => item.value === point.value);
      if (existing) existing.count++; else statusCounts.push({ value: point.value, count: 1 });
    }
    const kind = stats.count && statusCounts.length ? 'mixed' : stats.count ? 'numeric' : statusCounts.length ? 'categorical' : 'unavailable';
    return { key, name: event.name, label: resultMetricLabel(event.name), unit: event.unit, displayUnit, scope: event.scope, source: event.source,
      installation_id: first.installation_id, configuration_id: first.context.configuration_id ?? null, device: first.device,
      configuration: first.configuration, dataset_revision: first.dataset_revision, protocol_revision: first.protocol_revision,
      kind, points, rawPoints, stats, rawStats, sampleCount: rawPoints.length,
      unavailableCount: rawPoints.filter(point => point.value === null).length,
      operationCount: new Set(group.filter(item => item.context.attempt_id).map(item => item.context.attempt_id)).size,
      description: metricDescription(event.name, event.unit, event.scope), statusCounts, downsampled: points.length < rawPoints.length,
      omittedSegments: new Set(rawPoints.map(point => point.segment)).size - new Set(points.map(point => point.segment)).size,
    } satisfies ResultsMetric;
  }).sort((a, b) => a.label.localeCompare(b.label) || a.key.localeCompare(b.key));
}

export function buildResultsPresentation(projection: ObservationProjection, scope: ResultsScope = {}): ResultsPresentation {
  const selectedAttempts = projection.attempts.filter(item => inScope(item, scope));
  const selectedTurns = projection.turns.filter(item => inScope(item, { ...scope, attempt_id: undefined }) &&
    (!scope.attempt_id || selectedAttempts.some(attempt => attempt.installation_id === item.installation_id && attempt.context.turn_id === item.entity_id)));
  const turns = selectedTurns.map(turn => processingTurn(turn, selectedAttempts.filter(attempt =>
    attempt.installation_id === turn.installation_id && attempt.context.turn_id === turn.entity_id), projection, Boolean(scope.attempt_id)))
    .sort((a, b) => (a.turn_index ?? Infinity) - (b.turn_index ?? Infinity) || a.turn_id.localeCompare(b.turn_id));
  const conversations = projection.conversations.filter(item => inScope(item, { ...scope, turn_id: undefined, attempt_id: undefined }) &&
    ((!scope.turn_id && !scope.attempt_id) || selectedTurns.some(turn => turn.installation_id === item.installation_id && turn.context.conversation_id === item.entity_id)));
  const selectedMetrics = projection.raw_metrics.filter(item => inScope(item, scope));
  const timedEntities = scope.attempt_id ? selectedAttempts : scope.turn_id ? [...selectedAttempts, ...selectedTurns] : [...selectedAttempts, ...selectedTurns, ...conversations];
  const timestamps = timedEntities.flatMap(item => [item.started_at_ms, item.finished_at_ms]).filter(finite);
  for (const metric of selectedMetrics) timestamps.push(metric.event.timestamp_ms);
  const interval = { start_ms: timestamps.length ? timestamps.reduce((a, b) => Math.min(a, b)) : null,
    end_ms: timestamps.length ? timestamps.reduce((a, b) => Math.max(a, b)) : null };
  const metrics = metricSeries(selectedMetrics, interval.start_ms ?? 0);
  const missingTurnReasons = scope.turn_id || scope.attempt_id ? [] : conversations.flatMap(conversation => {
    const observed = turns.filter(turn => turn.installation_id === conversation.installation_id && turn.conversation_id === conversation.entity_id).length;
    return finite(conversation.data.expected_turns) && conversation.data.expected_turns > observed
      ? [`${conversation.data.expected_turns - observed} expected turns are absent from ${conversation.entity_id}.`] : [];
  });
  const recordedProcessing = total(turns.flatMap(turn => turn.segments), turns.flatMap(turn => turn.recordedProcessing.missing_stages),
    [...missingTurnReasons, ...turns.flatMap(turn => turn.recordedProcessing.reasons)]);
  const coverage = unique([
    ...(recordedProcessing.partial ? ['Recorded processing time is partial. Some expected durations or final collection evidence are missing.'] : []),
    ...(!metrics.some(metric => resourceNames.has(metric.name) && metric.kind !== 'unavailable') ? ['No measured resource history in this selection. Sampling may have been disabled.'] : []),
    ...(projection.coverage.dropped_events ? [`${projection.coverage.dropped_events} dropped events are recorded in this snapshot.`] : []),
    ...(projection.coverage.sequence_gaps.length ? ['This snapshot contains gaps in the recorded event sequence.'] : []),
    ...(projection.coverage.unresolved_reference_count ? ['Some observation parents are unavailable in this snapshot.'] : []),
    ...(metrics.some(metric => metric.omittedSegments) ? ['The compact graph omits some operation segments; every reading remains available in the data.'] : []),
  ]);
  const eligible = turns.filter(turn => turn.status === 'succeeded' && !turn.recordedProcessing.partial && turn.recordedProcessing.duration_ms !== null);
  // A mixed configuration or workload selection has no meaningful single typical duration.
  const populations = new Set(selectedTurns.map(turn => canonical([turn.device, turn.context.configuration_id, turn.dataset_revision,
    turn.protocol_revision, turn.source, turn.data.input_kind])));
  const typicalProcessingMs = eligible.length >= 2 && populations.size === 1 && !missingTurnReasons.length ? summarizeNumbers(eligible.map(turn => turn.recordedProcessing.duration_ms!)).median : null;
  return { scope: { ...scope }, turns, recordedProcessing, outcomes: counts(selectedTurns),
    lifecycleStatuses: conversations.map(item => ({ conversation_id: item.entity_id, status: item.status })), interval, metrics,
    memory: metrics.filter(metric => ['resident_memory_bytes', 'physical_footprint_bytes'].includes(metric.name) && metric.kind !== 'unavailable'),
    thermal: metrics.filter(metric => metric.name === 'thermal_state' && metric.kind !== 'unavailable'),
    capabilities: metrics.filter(metric => optionalNames.has(metric.name) && metric.kind === 'unavailable').map(metric => ({
      name: metric.name, label: metric.label, reasons: unique(metric.rawPoints.map(point => point.unavailable_reason ?? 'No measured provider result.')),
    })), coverage, typicalProcessingMs };
}

/** Structural subset of the persisted runner plan; no mobile runtime dependency. */
export interface PlannedSweep {
  id: string;
  jobs: readonly { id: string; configuration_index: number; configuration_id?: string; status: string; case_id?: string; repetition?: number }[];
  configurations: readonly { asr?: unknown; reasoning?: unknown; settings?: unknown; reasoning_threads?: number }[];
  protocol?: { mode?: string; conditions?: unknown };
  dataset?: { revision: string };
}
export interface SweepStageDistribution {
  key: string; timerBoundary: string | null; source: string; scope: string; unit: 'milliseconds';
  stats: NumericSummary; attemptCount: number; eligibleCount: number; outcomes: Record<string, number>;
  attempt_ids: string[]; record_references: RecordReference[];
}
export interface SweepConfigurationResult {
  key: string; configuration_id: string | null; configuration_ids: string[]; configuration_index: number | null;
  label: string; configuration: Record<string, unknown> | null; plannedJobs: number; outcomes: Record<string, number>;
  conversationCount: number; stages: Array<{ stage: string; label: string; groups: SweepStageDistribution[] }>;
  sampledPeakMemoryMiB: number | null; limitations: string[];
}
export interface SweepComparisonGroup {
  key: string; stage: string; engine: string | null; timerBoundary: string | null; source: string; scope: string; unit: 'milliseconds';
  device: PhoneDevice; dataset_revision: string | null; protocol_revision: string | null; mode: string | null;
  conditions: Record<string, unknown>; configurationKeys: string[]; limitations: string[];
}
export interface SweepComparison {
  sweep_id: string; plannedJobs: number; outcomes: Record<string, number>; configurations: SweepConfigurationResult[];
  groups: SweepComparisonGroup[]; limitations: string[];
}

/** Compare recorded executions within compatible populations. There is deliberately no model ranking. */
export function buildSweepComparison(projection: ObservationProjection, sweepID: string, plannedSweep?: PlannedSweep): SweepComparison {
  const plan = plannedSweep?.id === sweepID ? plannedSweep : undefined;
  const sweeps = projection.sweeps.filter(sweep => sweep.entity_id === sweepID);
  const jobs = projection.jobs.filter(job => job.context.sweep_id === sweepID);
  const attempts = projection.attempts.filter(attempt => attempt.context.sweep_id === sweepID);
  const conversations = projection.conversations.filter(conversation => conversation.context.sweep_id === sweepID);
  const sweepFor = (installation: string) => sweeps.find(sweep => sweep.installation_id === installation);
  const configurations = new Map<string, SweepConfigurationResult>();
  const jobKeys = new Map<string, string>();
  const mergedJobs: Array<{ observed?: EntityProjection; planned?: PlannedSweep['jobs'][number] }> = jobs.map(observed => ({
    observed, planned: plan?.jobs.find(job => job.id === observed.entity_id),
  }));
  for (const planned of plan?.jobs ?? []) if (!jobs.some(job => job.entity_id === planned.id)) mergedJobs.push({ planned });
  function configurationFor(installation: string, configurationID: string | undefined, index: number | null): SweepConfigurationResult {
    const observed = projection.configurations.find(configuration => configuration.installation_id === installation && configuration.entity_id === configurationID);
    const planned = index === null ? undefined : plan?.configurations[index];
    const metadata = observed?.data ?? (planned ? { ...planned } : null);
    // A planned matrix index does not prove two observed effective settings are
    // identical. Preserve separate rows if an artifact/runtime/settings changed.
    const variant = observed ? [observed.asr, observed.reasoning, observed.settings, observed.runtime, observed.role_sha256] : metadata;
    const key = canonical([installation, index === null ? configurationID ?? 'unattributed' : index, variant]);
    let result = configurations.get(key);
    if (!result) {
      const asr = object(metadata?.asr), reasoning = object(metadata?.reasoning);
      const names = [asr.name ?? asr.id, reasoning.name ?? reasoning.id].filter(value => typeof value === 'string');
      result = { key, configuration_id: configurationID ?? null, configuration_ids: configurationID ? [configurationID] : [], configuration_index: index,
        label: names.length ? names.join(' + ') : index === null ? 'Unattributed configuration' : `Configuration ${index + 1}`,
        configuration: metadata, plannedJobs: 0, outcomes: {}, conversationCount: 0, stages: [], sampledPeakMemoryMiB: null,
        limitations: metadata ? [] : ['Model artifacts and effective settings are unavailable.'] };
      configurations.set(key, result);
    } else if (configurationID && !result.configuration_ids.includes(configurationID)) {
      result.configuration_ids.push(configurationID);
      result.configuration_id ??= configurationID;
    }
    return result;
  }
  const allOutcomes: Record<string, number> = {};
  for (const { observed, planned } of mergedJobs) {
    const installation = observed?.installation_id ?? sweeps[0]?.installation_id ?? 'unknown';
    const index = planned?.configuration_index ?? (finite(observed?.data.configuration_index) ? observed.data.configuration_index : null);
    const result = configurationFor(installation, observed?.context.configuration_id ?? planned?.configuration_id, index);
    const status = observed?.status ?? planned?.status ?? 'unknown';
    result.plannedJobs++; result.outcomes[status] = (result.outcomes[status] ?? 0) + 1;
    allOutcomes[status] = (allOutcomes[status] ?? 0) + 1;
    jobKeys.set(identity(installation, observed?.entity_id ?? planned!.id), result.key);
  }
  const resultFor = (entity: EntityProjection) => {
    const jobKey = entity.context.job_id ? jobKeys.get(identity(entity.installation_id, entity.context.job_id)) : undefined;
    return jobKey ? configurations.get(jobKey)! : configurationFor(entity.installation_id, entity.context.configuration_id, null);
  };
  for (const conversation of conversations) resultFor(conversation).conversationCount++;
  const metricByAttempt = new Map<string, ProjectedMetric[]>();
  for (const metric of projection.raw_metrics) if (metric.context.sweep_id === sweepID && metric.context.attempt_id) {
    const key = identity(metric.installation_id, metric.context.attempt_id), existing = metricByAttempt.get(key) ?? [];
    existing.push(metric); metricByAttempt.set(key, existing);
  }
  const groups = new Map<string, SweepComparisonGroup>();
  const populations = new Map<string, { result: SweepConfigurationResult; group: SweepComparisonGroup; attempts: AttemptProjection[] }>();
  for (const attempt of attempts) {
    const result = resultFor(attempt), sweep = sweepFor(attempt.installation_id);
    const protocol = object(sweep?.data.protocol), plannedProtocol = plan?.protocol;
    const readings = metricByAttempt.get(identity(attempt.installation_id, attempt.entity_id)) ?? [];
    const conditionValues = (name: string) => unique(readings.filter(metric => metric.event.name === name && metric.event.value !== null)
      .map(metric => canonical([metric.event.value, metric.event.source, metric.event.scope, metric.event.unit]))).sort();
    const conditions = { protocol: protocol.conditions ?? plannedProtocol?.conditions ?? null,
      charging: conditionValues('battery_state'), power_mode: conditionValues('low_power_mode'), thermal: conditionValues('thermal_state') };
    const timerBoundary = string(attempt.data.timer_boundary);
    const source = string(attempt.data.duration_source) ?? (timerBoundary === 'native_call_including_bridge' ? 'phone.monotonic' : 'unknown');
    const metricScope = string(attempt.data.duration_scope) ?? 'native_call';
    const metadata = {
      stage: attempt.stage ?? 'unknown', engine: string(attempt.data.engine), timerBoundary, source, scope: metricScope, unit: 'milliseconds' as const,
      device: attempt.device, dataset_revision: attempt.dataset_revision ?? plan?.dataset?.revision ?? null,
      protocol_revision: attempt.protocol_revision, mode: string(attempt.data.mode ?? protocol.mode ?? plannedProtocol?.mode), conditions,
    };
    const key = canonical(metadata);
    let group = groups.get(key);
    if (!group) {
      const limits = [
        ...(!metadata.dataset_revision || !metadata.protocol_revision || !metadata.mode ? ['Dataset, protocol or mode identity is incomplete.'] : []),
        ...(!timerBoundary || source === 'unknown' ? ['The operation timing boundary or source is unknown.'] : []),
        ...(!conditions.charging.length || !conditions.power_mode.length || !conditions.thermal.length ? ['Charging, power-mode or thermal observations are missing.'] : []),
        ...(conditions.charging.length > 1 || conditions.power_mode.length > 1 || conditions.thermal.length > 1 ? ['Device conditions changed during these operations.'] : []),
      ];
      group = { key, ...metadata, configurationKeys: [], limitations: limits }; groups.set(key, group);
    }
    if (!group.configurationKeys.includes(result.key)) group.configurationKeys.push(result.key);
    const populationKey = canonical([key, result.key]), population = populations.get(populationKey) ?? { result, group, attempts: [] };
    population.attempts.push(attempt); populations.set(populationKey, population);
    for (const metric of readings) if (metric.event.name === 'resident_memory_bytes' && metric.event.unit === 'bytes' &&
      metric.event.scope === 'process' && finite(metric.event.value)) {
      const value = metric.event.value / (1024 * 1024);
      result.sampledPeakMemoryMiB = result.sampledPeakMemoryMiB === null ? value : Math.max(result.sampledPeakMemoryMiB, value);
    }
  }
  for (const { result, group, attempts: population } of populations.values()) {
    const eligible = population.filter(attempt => attempt.status === 'succeeded' && attempt.completeness === 'complete' && duration(attempt.duration_ms) !== null);
    const values = eligible.map(attempt => attempt.duration_ms!);
    const distribution: SweepStageDistribution = {
      key: group.key, timerBoundary: group.timerBoundary, source: group.source, scope: group.scope, unit: group.unit,
      stats: summarizeNumbers(values), attemptCount: population.length, eligibleCount: eligible.length, outcomes: counts(population),
      attempt_ids: population.map(attempt => attempt.entity_id), record_references: population.flatMap(attempt => attempt.record_references),
    };
    let stage = result.stages.find(stage => stage.stage === group.stage);
    if (!stage) { stage = { stage: group.stage, label: resultMetricLabel(group.stage), groups: [] }; result.stages.push(stage); }
    stage.groups.push(distribution);
    result.limitations = unique([...result.limitations, ...group.limitations]);
  }
  const plannedCount = sweeps.reduce((sum, sweep) => sum + (finite(sweep.data.planned_jobs) ? sweep.data.planned_jobs : 0), 0);
  const plannedJobs = Math.max(mergedJobs.length, plan?.jobs.length ?? 0, plannedCount);
  const limitations = [
    'Execution completion does not establish incident correctness or quality. These results do not recommend a model.',
    ...(groups.size > 1 ? ['Separate populations retain their device, dataset, protocol, timing and condition differences. Do not pool their medians.'] : []),
    ...(plannedJobs > mergedJobs.length ? [`${plannedJobs - mergedJobs.length} planned jobs have no retained job record or configuration identity.`] : []),
    ...unique([...groups.values()].flatMap(group => group.limitations)),
  ];
  return { sweep_id: sweepID, plannedJobs, outcomes: allOutcomes,
    configurations: [...configurations.values()].sort((a, b) => (a.configuration_index ?? Infinity) - (b.configuration_index ?? Infinity) || a.key.localeCompare(b.key)),
    groups: [...groups.values()], limitations };
}
