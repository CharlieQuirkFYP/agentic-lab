import type {
  AttemptProjection, Completeness, ConfigurationProjection, ConversationProjection, EntityProjection,
  EventReference, MetricGroup, ModelIdentity, NumericSummary, ObservationKind, ObservationProjection,
  PhoneMetricBatch, PhoneMetricEvent, PhoneObservationContext, ProjectedMetric,
  ProjectionOptions, RecordReference, SequenceGap, StoredPhoneObservation, TurnProjection, UnresolvedReference,
} from './types.ts';

export const P95_MINIMUM_COUNT = 20;
const entityFields: Partial<Record<ObservationKind, keyof PhoneObservationContext>> = {
  configuration: 'configuration_id', conversation: 'conversation_id', turn: 'turn_id',
  attempt: 'attempt_id', sweep: 'sweep_id', job: 'job_id',
};
const fieldKinds = Object.entries(entityFields).map(([kind, field]) => [field, kind] as const);
const executionKinds = ['conversation', 'turn', 'attempt', 'sweep', 'job'] as const;

export class ObservationConflictError extends Error {
  installation_id: string;
  identity: string;
  constructor(installationID: string, identity: string) {
    super(`Conflicting immutable phone observation: ${installationID} / ${identity}`);
    this.name = 'ObservationConflictError';
    this.installation_id = installationID;
    this.identity = identity;
  }
}

/** Canonical JSON makes key order irrelevant without changing scalar values. */
export function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    return `{${Object.keys(value).sort().filter(key => (value as Record<string, unknown>)[key] !== undefined)
      .map(key => `${JSON.stringify(key)}:${canonical((value as Record<string, unknown>)[key])}`).join(',')}}`;
  }
  return JSON.stringify(value) ?? 'null';
}
const key = (...parts: unknown[]) => canonical(parts);
const text = (value: unknown): string | null => typeof value === 'string' ? value : null;
const number = (value: unknown): number | null => typeof value === 'number' && Number.isFinite(value) ? value : null;
const object = (value: unknown): Record<string, unknown> => value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
const reference = (stored: StoredPhoneObservation): RecordReference => ({ installation_id: stored.device.installation_id, record_id: stored.record.record_id });
const same = (a: unknown, b: unknown) => canonical(a) === canonical(b);
const sorted = <T>(items: T[], identify: (item: T) => string): T[] => items.sort((a, b) => identify(a).localeCompare(identify(b)));
const entityKey = (installation: string, kind: string, id: string) => key(installation, kind, id);

/** All finite samples contribute once. Percentile eligibility is explicit. */
export function summarizeNumbers(values: readonly number[], eligibleValues: readonly number[] = values): NumericSummary {
  const finite = values.filter(Number.isFinite).sort((a, b) => a - b);
  const eligible = eligibleValues.filter(Number.isFinite).sort((a, b) => a - b);
  const count = finite.length;
  const sum = count ? finite.reduce((a, b) => a + b, 0) : null;
  return {
    count, sum, mean: sum === null ? null : sum / count, min: finite[0] ?? null, max: finite[count - 1] ?? null,
    median: count ? count % 2 ? finite[Math.floor(count / 2)] : (finite[count / 2 - 1] + finite[count / 2]) / 2 : null,
    p95: eligible.length >= P95_MINIMUM_COUNT ? eligible[Math.ceil(eligible.length * 0.95) - 1] : null,
    p95_eligible_count: eligible.length, p95_minimum_count: 20,
  };
}

function deduplicate(input: readonly StoredPhoneObservation[]): { records: StoredPhoneObservation[]; duplicates: number } {
  const identities = new Map<string, string>();
  const records = new Map<string, StoredPhoneObservation>();
  let duplicates = 0;
  for (const stored of input) {
    const { record, device } = stored;
    if (record.schema_version !== 1 || !device.installation_id || !record.record_id ||
      !record.producer_stream_id || !record.entity_id || !Number.isSafeInteger(record.record_sequence) ||
      !Number.isSafeInteger(record.revision) || record.record_sequence < 0 || record.revision < 0 ||
      !Number.isFinite(record.timestamp_ms) || !Number.isFinite(stored.received_at_ms)) {
      throw new TypeError('Invalid canonical phone observation envelope');
    }
    const fingerprint = canonical(record);
    for (const identity of [key('record', record.record_id), key('stream', record.producer_stream_id, record.record_sequence),
      key('revision', record.kind, record.entity_id, record.revision)]) {
      const scoped = key(device.installation_id, identity);
      const previous = identities.get(scoped);
      if (previous !== undefined && previous !== fingerprint) throw new ObservationConflictError(device.installation_id, identity);
      identities.set(scoped, fingerprint);
    }
    const id = key(device.installation_id, record.record_id);
    const previous = records.get(id);
    if (previous) {
      duplicates++;
      // Retry receipt time must not make an immutable snapshot appear newer.
      if (stored.received_at_ms < previous.received_at_ms) records.set(id, stored);
    } else records.set(id, stored);
  }
  return { records: sorted([...records.values()], item => key(item.device.installation_id, item.record.producer_stream_id, item.record.record_sequence, item.record.record_id)), duplicates };
}

function sequenceGaps(records: StoredPhoneObservation[]): SequenceGap[] {
  const streams = new Map<string, { installation_id: string; producer_stream_id: string; sequences: number[] }>();
  for (const { device, record } of records) {
    const id = key(device.installation_id, record.producer_stream_id);
    const stream = streams.get(id) ?? { installation_id: device.installation_id, producer_stream_id: record.producer_stream_id, sequences: [] };
    stream.sequences.push(record.record_sequence); streams.set(id, stream);
  }
  return [...streams.values()].flatMap(stream => {
    const sequences = stream.sequences.sort((a, b) => a - b);
    return sequences.flatMap((sequence, index) => index > 0 && sequence > sequences[index - 1] + 1 ? [{
      installation_id: stream.installation_id, producer_stream_id: stream.producer_stream_id,
      after_sequence: sequences[index - 1], before_sequence: sequence, missing_count: sequence - sequences[index - 1] - 1,
    }] : []);
  });
}

function model(value: unknown): ModelIdentity | null {
  if (value === null || value === undefined) return null;
  const data = object(value);
  return { id: text(data.id), name: text(data.name), sha256: text(data.sha256), metadata: data };
}

function metricBatch(value: unknown): PhoneMetricBatch {
  const batch = object(value);
  if (batch.schema_version !== 1 || typeof batch.run_id !== 'string' || !Array.isArray(batch.events)) {
    throw new TypeError('Invalid phone observation metric batch');
  }
  for (const raw of batch.events) {
    const event = object(raw);
    if (event.schema_version !== 1 || typeof event.run_id !== 'string' || !Number.isSafeInteger(event.sequence) ||
      !Number.isFinite(event.timestamp_ms) || !['name', 'unit', 'scope', 'source'].every(field => typeof event[field] === 'string') ||
      !(event.value === null || typeof event.value === 'string' || typeof event.value === 'boolean' || number(event.value) !== null)) {
      throw new TypeError('Invalid phone observation metric event');
    }
  }
  return batch as unknown as PhoneMetricBatch;
}

export function groupMetrics(metrics: readonly ProjectedMetric[]): MetricGroup[] {
  const groups = new Map<string, { group: MetricGroup; values: number[]; eligible: number[] }>();
  for (const metric of metrics) {
    const { event, context, device } = metric;
    const identity = {
      installation_id: metric.installation_id, configuration_id: context.configuration_id ?? null,
      configuration: metric.configuration,
      hardware: device.hardware, os_version: device.os_version, app_version: device.app_version, build: device.build,
      dataset_revision: metric.dataset_revision, protocol_revision: metric.protocol_revision,
      stage: metric.stage, name: event.name, unit: event.unit, scope: event.scope, source: event.source, outcome: metric.outcome,
    };
    const id = canonical(identity);
    let entry = groups.get(id);
    if (!entry) {
      entry = { group: { key: id, ...identity, numeric: summarizeNumbers([]), status_counts: [], unavailable_count: 0,
        unavailable_reasons: [], event_count: 0, event_references: [] }, values: [], eligible: [] };
      groups.set(id, entry);
    }
    entry.group.event_count++;
    entry.group.event_references.push(...metric.references);
    if (typeof event.value === 'number' && Number.isFinite(event.value)) {
      entry.values.push(event.value);
      // Raw events include resource polling and repeated stage timings. Only
      // one recorded entity duration contributes to duration p95 in reports.
    } else if (event.value === null) {
      entry.group.unavailable_count++;
      const reason = event.unavailable_reason ?? null;
      const existing = entry.group.unavailable_reasons.find(item => item.reason === reason);
      if (existing) existing.count++; else entry.group.unavailable_reasons.push({ reason, count: 1 });
    } else if (typeof event.value === 'string' || typeof event.value === 'boolean') {
      const existing = entry.group.status_counts.find(item => item.value === event.value);
      if (existing) existing.count++; else entry.group.status_counts.push({ value: event.value, count: 1 });
    }
  }
  return sorted([...groups.values()].map(({ group, values, eligible }) => ({ ...group, numeric: summarizeNumbers(values, eligible) })), group => group.key);
}

/** Resolve all selected records together; never aggregate a visible page only. */
export function projectObservations(input: readonly StoredPhoneObservation[], options: ProjectionOptions = {}): ObservationProjection {
  const { records, duplicates } = deduplicate(input);
  const latest = new Map<string, StoredPhoneObservation>();
  const history = new Map<string, RecordReference[]>();
  for (const stored of records) {
    const id = entityKey(stored.device.installation_id, stored.record.kind, stored.record.entity_id);
    const previous = latest.get(id);
    if (!previous || stored.record.revision > previous.record.revision) latest.set(id, stored);
    const refs = history.get(id) ?? []; refs.push(reference(stored)); history.set(id, refs);
  }
  const resolved = new Map<string, PhoneObservationContext>();
  function resolve(stored: StoredPhoneObservation, visiting = new Set<string>()): PhoneObservationContext {
    const id = entityKey(stored.device.installation_id, stored.record.kind, stored.record.entity_id);
    const cached = resolved.get(id); if (cached) return cached;
    if (visiting.has(id)) throw new ObservationConflictError(stored.device.installation_id, 'cyclic entity relationship');
    visiting.add(id);
    let context: PhoneObservationContext = {};
    for (const [field, kind] of fieldKinds) {
      const parentID = stored.record.context[field];
      if (!parentID || kind === stored.record.kind && parentID === stored.record.entity_id) continue;
      const parent = latest.get(entityKey(stored.device.installation_id, kind, parentID));
      if (!parent) continue;
      const parentContext = resolve(parent, visiting);
      for (const [parentField, value] of Object.entries(parentContext)) {
        const prior = context[parentField as keyof PhoneObservationContext];
        if (prior && value && prior !== value) throw new ObservationConflictError(stored.device.installation_id, `contradictory ${parentField}`);
      }
      context = { ...context, ...parentContext };
    }
    for (const [field, value] of Object.entries(stored.record.context)) {
      const prior = context[field as keyof PhoneObservationContext];
      if (prior && value && prior !== value) throw new ObservationConflictError(stored.device.installation_id, `contradictory ${field}`);
    }
    context = { ...context, ...stored.record.context };
    const ownField = entityFields[stored.record.kind];
    if (ownField) {
      if (context[ownField] && context[ownField] !== stored.record.entity_id) throw new ObservationConflictError(stored.device.installation_id, `contradictory ${ownField}`);
      context[ownField] = stored.record.entity_id;
    }
    visiting.delete(id); resolved.set(id, context); return context;
  }
  const unresolved: UnresolvedReference[] = [];
  for (const stored of latest.values()) {
    const context = resolve(stored);
    for (const [field, kind] of fieldKinds) {
      const id = context[field];
      if (id && !latest.has(entityKey(stored.device.installation_id, kind, id))) unresolved.push({ ...reference(stored), field, entity_id: id });
    }
    if (stored.record.kind === 'metric_batch' && !context.attempt_id) {
      unresolved.push({ ...reference(stored), field: 'attempt_id', entity_id: '' });
    }
  }
  const gaps = options.filtered ? [] : sequenceGaps(records);
  const recordsByID = new Map(records.map(item => [key(item.device.installation_id, item.record.record_id), item]));
  const batchIdentity = (stored: StoredPhoneObservation, batch: PhoneMetricBatch): string => {
    const context = resolve(stored);
    return key(stored.device.installation_id, context.attempt_id ?? null, context.attempt_id ? null : context.runtime_session_id ?? null, batch.run_id, batch.sequence_number);
  };
  function distinctBatches(items: StoredPhoneObservation[]): Map<string, PhoneMetricBatch> {
    const batches = new Map<string, PhoneMetricBatch>();
    for (const item of items) {
      if (item.record.kind !== 'metric_batch') continue;
      const batch = metricBatch(item.record.data.batch);
      const identity = batchIdentity(item, batch);
      const previous = batches.get(identity);
      if (previous && !same(previous, batch)) throw new ObservationConflictError(item.device.installation_id, `native batch ${identity}`);
      batches.set(identity, batch);
    }
    return batches;
  }
  const uniqueMetricBatches = distinctBatches([...latest.values()]);
  const metadata = (stored: StoredPhoneObservation, field: string): string | null => {
    if (text(stored.record.data[field]) !== null) return text(stored.record.data[field]);
    const context = resolve(stored);
    const nearestParents = [['attempt_id', 'attempt'], ['turn_id', 'turn'], ['conversation_id', 'conversation'],
      ['job_id', 'job'], ['sweep_id', 'sweep'], ['configuration_id', 'configuration']] as const;
    for (const [idField, kind] of nearestParents) {
      const id = context[idField]; if (!id) continue;
      const parent = latest.get(entityKey(stored.device.installation_id, kind, id));
      if (parent && text(parent.record.data[field]) !== null) return text(parent.record.data[field]);
    }
    return null;
  };
  const belongs = (stored: StoredPhoneObservation, kind: ObservationKind, id: string): boolean => {
    const field = entityFields[kind];
    return !!field && resolve(stored)[field] === id;
  };
  const finalizations = [...latest.values()].filter(stored => stored.record.kind === 'finalization');
  const entities: EntityProjection[] = [];
  for (const stored of latest.values()) {
    const { record, device } = stored;
    if (!entityFields[record.kind]) continue;
    const context = resolve(stored);
    const relevant = [...latest.values()].filter(item => item.device.installation_id === device.installation_id && belongs(item, record.kind, record.entity_id));
    // Only a finalization directly targeting the entity can certify it complete.
    const final = finalizations.filter(item => item.device.installation_id === device.installation_id && belongs(item, record.kind, record.entity_id) &&
      (record.kind === 'attempt' || !resolve(item).attempt_id) &&
      (record.kind !== 'sweep' || !resolve(item).conversation_id))
      .sort((a, b) => b.record.timestamp_ms - a.record.timestamp_ms || b.record.revision - a.record.revision)[0];
    const dropped = final ? number(final.record.data.dropped_events) ?? 0 : 0;
    const eventCount = final ? number(final.record.data.event_count) : null;
    const observedEvents = [...distinctBatches(relevant).values()].reduce((sum, batch) => sum + batch.events.length, 0);
    const hasUnresolved = relevant.some(item => unresolved.some(ref => ref.installation_id === device.installation_id && ref.record_id === item.record.record_id));
    const hasGap = relevant.some(item => gaps.some(gap => gap.installation_id === device.installation_id && gap.producer_stream_id === item.record.producer_stream_id &&
      relevant.some(other => other.record.producer_stream_id === gap.producer_stream_id && other.record.record_sequence <= gap.after_sequence) &&
      relevant.some(other => other.record.producer_stream_id === gap.producer_stream_id && other.record.record_sequence >= gap.before_sequence)));
    const expectedRecords = final ? number(final.record.data.record_count) : null;
    const requiredIDs = final && Array.isArray(final.record.data.required_record_ids) && final.record.data.required_record_ids.every(id => typeof id === 'string')
      ? final.record.data.required_record_ids as string[] : null;
    const expectedTargetRevision = final ? number(final.record.data.target_revision) : null;
    const requiredSet = new Set(requiredIDs ?? []);
    const coveredRecords = records.filter(item => item.device.installation_id === device.installation_id && item.record.kind !== 'finalization' && belongs(item, record.kind, record.entity_id));
    const evidenceValid = requiredIDs !== null && requiredSet.size === requiredIDs.length && expectedRecords === requiredIDs.length &&
      expectedTargetRevision === record.revision && requiredSet.has(record.record_id) && eventCount !== null &&
      number(final?.record.data.dropped_events) !== null &&
      requiredIDs.every(id => recordsByID.has(key(device.installation_id, id)) && !unresolved.some(ref => ref.installation_id === device.installation_id && ref.record_id === id)) &&
      coveredRecords.every(item => requiredSet.has(item.record.record_id));
    const incomplete = dropped > 0 || hasUnresolved || hasGap || eventCount !== null && eventCount !== observedEvents ||
      final !== undefined && !evidenceValid;
    const completeness: Completeness = incomplete || final?.record.data.status === 'partial' ? 'partial' : final?.record.data.status === 'complete' ? 'complete' : 'unknown';
    entities.push({
      installation_id: device.installation_id, device, entity_id: record.entity_id, kind: record.kind, revision: record.revision,
      record_id: record.record_id, context, data: record.data, status: text(record.data.status), source: metadata(stored, 'source'), stage: metadata(stored, 'stage'),
      dataset_revision: metadata(stored, 'dataset_revision'), protocol_revision: metadata(stored, 'protocol_revision'),
      started_at_ms: number(record.data.started_at_ms), finished_at_ms: number(record.data.finished_at_ms), duration_ms: number(record.data.duration_ms),
      completeness, dropped_events: dropped, event_count: eventCount, record_references: history.get(entityKey(device.installation_id, record.kind, record.entity_id)) ?? [], metric_groups: [],
    });
  }
  const attempts: AttemptProjection[] = entities.filter(entity => entity.kind === 'attempt').map(entity => ({ ...entity, native_run_id: text(entity.data.native_run_id) }));
  const metricIdentities = new Map<string, ProjectedMetric>();
  for (const stored of latest.values()) {
    if (stored.record.kind !== 'metric_batch') continue;
    const context = resolve(stored);
    const attempt = attempts.find(item => item.installation_id === stored.device.installation_id && item.entity_id === context.attempt_id);
    const batch = metricBatch(stored.record.data.batch);
    batch.events.forEach((event: PhoneMetricEvent, eventIndex: number) => {
      const id = key(stored.device.installation_id, context.attempt_id ?? stored.record.entity_id, context.runtime_session_id ?? null, event.run_id, event.source, event.sequence);
      const ref: EventReference = { ...reference(stored), event_index: eventIndex, run_id: event.run_id, sequence: event.sequence };
      const previous = metricIdentities.get(id);
      if (previous) {
        if (!same(previous.event, event)) throw new ObservationConflictError(stored.device.installation_id, `metric event ${id}`);
        previous.references.push(ref);
      } else metricIdentities.set(id, { event, context, installation_id: stored.device.installation_id, device: attempt?.device ?? stored.device,
        configuration: context.configuration_id ? latest.get(entityKey(stored.device.installation_id, 'configuration', context.configuration_id))?.record.data ?? null : null,
        stage: attempt?.stage ?? metadata(stored, 'stage'), outcome: attempt?.status ?? null, completeness: attempt?.completeness ?? 'unknown',
        dataset_revision: metadata(stored, 'dataset_revision'), protocol_revision: metadata(stored, 'protocol_revision'), references: [ref] });
    });
  }
  const rawMetrics = [...metricIdentities.values()];
  const entityMetrics = (entity: EntityProjection) => rawMetrics.filter(metric => metric.installation_id === entity.installation_id && metric.context[entityFields[entity.kind]!] === entity.entity_id);
  for (const entity of [...entities, ...attempts]) entity.metric_groups = groupMetrics(entityMetrics(entity));
  const turns: TurnProjection[] = entities.filter(entity => entity.kind === 'turn').map(entity => ({ ...entity, turn_index: number(entity.data.turn_index), attempts: attempts.filter(attempt => attempt.installation_id === entity.installation_id && attempt.context.turn_id === entity.entity_id) }));
  const conversations: ConversationProjection[] = entities.filter(entity => entity.kind === 'conversation').map(entity => ({ ...entity,
    turns: turns.filter(turn => turn.installation_id === entity.installation_id && turn.context.conversation_id === entity.entity_id).sort((a, b) => (a.turn_index ?? Infinity) - (b.turn_index ?? Infinity) || a.entity_id.localeCompare(b.entity_id)),
    attempts: attempts.filter(attempt => attempt.installation_id === entity.installation_id && attempt.context.conversation_id === entity.entity_id),
  }));
  for (const turn of turns) {
    if (turn.attempts.some(attempt => attempt.completeness === 'partial')) turn.completeness = 'partial';
    else if (turn.completeness === 'unknown' && turn.attempts.length && ['succeeded', 'failed', 'cancelled', 'interrupted', 'skipped'].includes(turn.status ?? '') &&
      turn.attempts.every(attempt => attempt.completeness === 'complete')) turn.completeness = 'complete';
  }
  for (const conversation of conversations) {
    if (conversation.attempts.some(attempt => attempt.completeness === 'partial')) conversation.completeness = 'partial';
    else if (conversation.completeness === 'complete' && conversation.attempts.some(attempt => attempt.completeness !== 'complete')) conversation.completeness = 'partial';
  }
  const configurations: ConfigurationProjection[] = entities.filter(entity => entity.kind === 'configuration').map(entity => ({ ...entity,
    asr: model(entity.data.asr), reasoning: model(entity.data.reasoning), settings: object(entity.data.settings), runtime: object(entity.data.runtime), role_sha256: text(entity.data.role_sha256) }));
  const outcomes = Object.fromEntries(executionKinds.map(kind => {
    const counts: Record<string, number> = {};
    for (const entity of entities.filter(item => item.kind === kind)) counts[entity.status ?? 'unknown'] = (counts[entity.status ?? 'unknown'] ?? 0) + 1;
    return [kind, counts];
  })) as ObservationProjection['counts']['outcomes'];
  const completeness = { complete: 0, partial: 0, unknown: 0 };
  for (const entity of conversations) completeness[entity.completeness]++;
  const extent = (values: number[], edge: 'min' | 'max'): number | null => values.length ? values.reduce((a, b) => edge === 'min' ? Math.min(a, b) : Math.max(a, b)) : null;
  const timestamps = records.map(stored => stored.record.timestamp_ms);
  const receiptTimes = records.map(stored => stored.received_at_ms);
  const sweeps = entities.filter(entity => entity.kind === 'sweep');
  for (const sweep of sweeps) {
    const children = conversations.filter(conversation => conversation.installation_id === sweep.installation_id && conversation.context.sweep_id === sweep.entity_id);
    const childAttempts = attempts.filter(attempt => attempt.installation_id === sweep.installation_id && attempt.context.sweep_id === sweep.entity_id);
    if ([...children, ...childAttempts].some(child => child.completeness === 'partial') || sweep.completeness === 'complete' &&
      [...children, ...childAttempts].some(child => child.completeness !== 'complete')) sweep.completeness = 'partial';
  }
  const jobs = entities.filter(entity => entity.kind === 'job');
  // Parent counters may repeat their children's drops. Preserve the greatest
  // recorded parent total or sum of disjoint children, without counting both.
  const dropNodes = new Map<string, { own: number; timestamp: number; parent: string | null; children: Set<string> }>();
  const dropNode = (id: string) => {
    let node = dropNodes.get(id);
    if (!node) { node = { own: 0, timestamp: -Infinity, parent: null, children: new Set() }; dropNodes.set(id, node); }
    return node;
  };
  for (const final of finalizations) {
    const context = resolve(final);
    const hierarchy = ([['sweep', context.sweep_id], ['conversation', context.conversation_id], ['attempt', context.attempt_id]] as const)
      .filter((entry): entry is readonly ['sweep' | 'conversation' | 'attempt', string] => entry[1] !== undefined)
      .map(([kind, id]) => entityKey(final.device.installation_id, kind, id));
    if (!hierarchy.length) hierarchy.push(entityKey(final.device.installation_id, 'finalization', final.record.entity_id));
    hierarchy.forEach((id, index) => {
      const node = dropNode(id);
      if (index > 0) { node.parent = hierarchy[index - 1]; dropNode(node.parent).children.add(id); }
    });
    const target = dropNode(hierarchy[hierarchy.length - 1]);
    if (final.record.timestamp_ms >= target.timestamp) { target.own = number(final.record.data.dropped_events) ?? 0; target.timestamp = final.record.timestamp_ms; }
  }
  const subtreeDrops = (id: string): number => {
    const node = dropNodes.get(id)!;
    return Math.max(node.own, [...node.children].reduce((sum, child) => sum + subtreeDrops(child), 0));
  };
  const droppedEvents = [...dropNodes].filter(([, node]) => node.parent === null).reduce((sum, [id]) => sum + subtreeDrops(id), 0);
  return { schema_version: 1, records, configurations, conversations, turns, attempts, sweeps, jobs,
    metric_groups: groupMetrics(rawMetrics), raw_metrics: rawMetrics, unresolved_references: unresolved,
    counts: { records: records.length, installations: new Set(records.map(item => item.device.installation_id)).size,
      configurations: configurations.length, conversations: conversations.length, turns: turns.length, attempts: attempts.length,
      sweeps: sweeps.length, jobs: jobs.length, metric_batches: uniqueMetricBatches.size,
      metric_events: rawMetrics.length, outcomes, completeness },
    coverage: { first_timestamp_ms: extent(timestamps, 'min'), last_timestamp_ms: extent(timestamps, 'max'),
      last_received_at_ms: extent(receiptTimes, 'max'), duplicate_records: duplicates,
      dropped_events: droppedEvents, sequence_gaps: gaps, unresolved_reference_count: unresolved.length,
      completeness: gaps.length || unresolved.length || droppedEvents || completeness.partial ? 'partial' : conversations.length && completeness.complete === conversations.length ? 'complete' : 'unknown' },
  };
}
