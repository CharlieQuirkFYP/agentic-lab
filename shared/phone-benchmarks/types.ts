/** Canonical wire and projection types. No runtime or platform dependencies. */
export type ObservationKind = 'configuration' | 'conversation' | 'turn' | 'attempt' | 'metric_batch' | 'sweep' | 'job' | 'finalization';
export type ExecutionStatus = 'planned' | 'active' | 'running' | 'succeeded' | 'failed' | 'cancelled' | 'interrupted' | 'skipped';
export type ObservationSource = 'manual' | 'sweep' | 'acceptance';
export type AttemptStage = 'transcription' | 'reasoning' | 'loading' | 'warmup';
export type Completeness = 'complete' | 'partial' | 'unknown';
export interface PhoneDevice {
  installation_id: string; hardware: string; os_version: string; app_version: string; build: string;
}
export interface PhoneObservationContext {
  configuration_id?: string; conversation_id?: string; turn_id?: string; attempt_id?: string;
  sweep_id?: string; job_id?: string; runtime_session_id?: string;
}
export type ObservationContext = PhoneObservationContext;
export interface PhoneObservation {
  schema_version: 1; record_id: string; producer_stream_id: string; record_sequence: number;
  timestamp_ms: number; kind: ObservationKind; entity_id: string; revision: number;
  context: PhoneObservationContext; data: Record<string, unknown>;
}
export interface StoredPhoneObservation { device: PhoneDevice; record: PhoneObservation; received_at_ms: number }
export interface PhoneObservationUpload { schema_version: 1; device: PhoneDevice; records: PhoneObservation[] }
export interface PhoneObservationPage {
  schema_version: 1; as_of: string; records: StoredPhoneObservation[]; total: number; next_cursor: string | null;
  filtered?: boolean; selected_record_keys?: RecordReference[];
}
export type MetricValue = number | string | boolean | null;
export interface PhoneMetricEvent {
  schema_version: 1; experiment_id?: string; run_id: string; sequence: number; timestamp_ms: number;
  name: string; value: MetricValue; unit: string; scope: string; source: string; unavailable_reason?: string;
}
export interface PhoneMetricBatch {
  schema_version: 1; experiment_id?: string; run_id: string; sequence_number: number; timestamp_ms: number; events: PhoneMetricEvent[];
}
export interface ModelIdentity { id: string | null; name: string | null; sha256: string | null; metadata: Record<string, unknown> }
export interface RecordReference { installation_id: string; record_id: string }
export interface EventReference extends RecordReference { event_index: number; run_id: string; sequence: number }
export interface NumericSummary {
  count: number; sum: number | null; mean: number | null; min: number | null; max: number | null;
  median: number | null; p95: number | null; p95_eligible_count: number; p95_minimum_count: 20;
}
export interface MetricGroup {
  key: string; installation_id: string; configuration_id: string | null; stage: string | null;
  configuration: Record<string, unknown> | null;
  hardware: string; os_version: string; app_version: string; build: string;
  dataset_revision: string | null; protocol_revision: string | null;
  name: string; unit: string; scope: string; source: string; outcome: string | null;
  numeric: NumericSummary; status_counts: Array<{ value: string | boolean; count: number }>;
  unavailable_count: number; unavailable_reasons: Array<{ reason: string | null; count: number }>;
  event_count: number; event_references: EventReference[];
}
export interface ProjectedMetric {
  event: PhoneMetricEvent; context: PhoneObservationContext; installation_id: string;
  device: PhoneDevice; dataset_revision: string | null; protocol_revision: string | null;
  configuration: Record<string, unknown> | null;
  /** Explicit historical links, without synthesized conversation identities. */
  parent_run_id?: string | null; native_run_id?: string | null;
  stage: string | null; outcome: string | null; completeness: Completeness; references: EventReference[];
}
export interface EntityProjection {
  installation_id: string; entity_id: string; kind: ObservationKind; revision: number;
  device: PhoneDevice; dataset_revision: string | null; protocol_revision: string | null;
  record_id: string; context: PhoneObservationContext; data: Record<string, unknown>;
  status: string | null; source: string | null; stage: string | null;
  started_at_ms: number | null; finished_at_ms: number | null; duration_ms: number | null;
  completeness: Completeness; dropped_events: number; event_count: number | null;
  record_references: RecordReference[]; metric_groups: MetricGroup[];
}
export interface ConfigurationProjection extends EntityProjection {
  asr: ModelIdentity | null; reasoning: ModelIdentity | null;
  settings: Record<string, unknown>; runtime: Record<string, unknown>; role_sha256: string | null;
}
export interface AttemptProjection extends EntityProjection { native_run_id: string | null }
export interface TurnProjection extends EntityProjection { turn_index: number | null; attempts: AttemptProjection[] }
export interface ConversationProjection extends EntityProjection { turns: TurnProjection[]; attempts: AttemptProjection[] }
export interface SequenceGap { installation_id: string; producer_stream_id: string; after_sequence: number; before_sequence: number; missing_count: number }
export interface UnresolvedReference extends RecordReference { field: keyof PhoneObservationContext; entity_id: string }
export interface ObservationCounts {
  records: number; configurations: number; conversations: number; turns: number; attempts: number;
  sweeps: number; jobs: number; metric_batches: number; metric_events: number; installations: number;
  outcomes: Record<'conversation' | 'turn' | 'attempt' | 'sweep' | 'job', Record<string, number>>;
  completeness: Record<Completeness, number>;
}
export interface ObservationCoverage {
  first_timestamp_ms: number | null; last_timestamp_ms: number | null; last_received_at_ms: number | null;
  duplicate_records: number; dropped_events: number; sequence_gaps: SequenceGap[];
  unresolved_reference_count: number; completeness: Completeness;
}
export interface ObservationProjection {
  schema_version: 1; records: StoredPhoneObservation[]; configurations: ConfigurationProjection[];
  conversations: ConversationProjection[]; turns: TurnProjection[]; attempts: AttemptProjection[];
  sweeps: EntityProjection[]; jobs: EntityProjection[]; metric_groups: MetricGroup[];
  raw_metrics: ProjectedMetric[]; counts: ObservationCounts; coverage: ObservationCoverage;
  unresolved_references: UnresolvedReference[];
}
export interface ProjectionOptions {
  /** A filtered snapshot may intentionally omit unrelated producer sequence positions. */
  filtered?: boolean;
  selected_record_keys?: readonly RecordReference[];
}
export interface ObservationFilters {
  installation_id?: string; configuration_id?: string; conversation_id?: string; sweep_id?: string;
  since_ms?: number; until_ms?: number;
  source?: string; stage?: string; outcome?: string;
}
export interface DurationGroup {
  installation_id: string; configuration_id: string | null; entity_kind: 'conversation' | 'turn' | 'attempt';
  stage: string | null; source: string | null; outcome: string | null; numeric: NumericSummary;
  hardware: string; os_version: string; app_version: string; build: string;
  dataset_revision: string | null; protocol_revision: string | null;
  record_references: RecordReference[];
}
export interface BenchmarkReport {
  schema_version: 1; report_version: 2; generated_at_ms: number;
  input: { as_of: string; filters: ObservationFilters; record_count: number; record_references: RecordReference[]; selected_record_keys: RecordReference[]; filtered: boolean };
  methodology: { p95_minimum_count: 20; percentile_method: 'nearest_rank'; duration_unit: 'milliseconds'; p95_eligibility: string; coverage: string };
  projection: ObservationProjection; duration_groups: DurationGroup[];
  legacy: LegacyProjection; metric_groups: MetricGroup[];
  totals: { canonical_records: number; legacy_batches: number; metric_batches: number; metric_events: number; conversations: number; turns: number; attempts: number };
}
export interface LegacyPhoneObservation { device: PhoneDevice | null; batch: PhoneMetricBatch; received_at_ms: number }
export interface LegacyProjection {
  records: LegacyPhoneObservation[]; raw_metrics: ProjectedMetric[]; metric_groups: MetricGroup[];
  batch_count: number; event_count: number; duplicate_batches: number; conversation_count: null;
  canonical_duplicate_batches: number;
  note: string;
}
export interface ReportOptions {
  generated_at_ms: number; as_of: string; filters?: ObservationFilters;
  legacy_records?: readonly LegacyPhoneObservation[];
  /** API applies filters and includes relationship ancestors. Do not filter those ancestors again. */
  input_already_filtered?: boolean;
  selected_record_keys?: readonly RecordReference[];
}
