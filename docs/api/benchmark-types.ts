// Frontend contract reference for the Go benchmark API. See benchmark-ui.md.
// The current server uses mock execution; these types do not imply real inference.
export type ExperimentStatus = 'queued' | 'running' | 'completed' | 'failed';
export type ResultSource = 'mock' | 'measured' | 'unavailable';
export type UseCase = 'incident-reporting' | 'interview-assistant' | 'license-plate-monitoring';

export interface ModelConfig {
  name?: string;
  version?: string;
  quantization?: string;
  runtime?: string;
}

export interface EnvironmentProfile {
  name?: string;
  type?: string;
  cpu_cores?: number;
  memory_mb?: number;
  gpu_info?: string;
}

export interface CreateExperimentRequest {
  use_case: UseCase;
  dataset?: string;
  model_config?: ModelConfig;
  environment_profile?: EnvironmentProfile;
}

export interface CreateExperimentResponse { id: string; status: 'queued' }

export interface ExperimentResult {
  latency_ms: number;
  peak_memory_mb: number;
  tokens_per_second?: number;
  success: boolean;
  error_message?: string;
}

export interface Experiment {
  id: string;
  use_case: UseCase;
  dataset?: string;
  model_config: ModelConfig;
  environment_profile: EnvironmentProfile;
  result_source: ResultSource;
  status: ExperimentStatus;
  created_at: string;
  started_at?: string;
  completed_at?: string;
  result?: ExperimentResult;
}

export interface Pagination { total: number; limit: number; offset: number }
export interface ExperimentPage extends Pagination { experiments: Experiment[] }

export interface MetricEvent {
  schema_version: 1;
  experiment_id?: string;
  run_id: string;
  sequence: number;
  timestamp_ms: number;
  name: string;
  value: number | string | boolean | null;
  unit: string;
  scope: string;
  source: string;
  unavailable_reason?: string; // Required exactly when value is null.
}

export interface MetricBatch {
  schema_version: 1;
  experiment_id?: string;
  run_id: string;
  sequence_number: number;
  timestamp_ms: number;
  events: MetricEvent[];
}

export interface MetricPage extends Pagination {
  experiment_id: string;
  batches: MetricBatch[];
}

export interface MetricSummary {
  name: string;
  unit: string;
  scope: string;
  source: string;
  observation_count: number;
  numeric_count: number;
  unavailable_count: number;
  unavailable_reasons: string[];
  min: number | null;
  max: number | null;
  mean: number | null;
}

export interface RunSummary {
  run_id: string;
  batch_count: number;
  event_count: number;
  first_timestamp_ms: number;
  last_timestamp_ms: number;
  metrics: MetricSummary[];
}

export interface RunPage extends Pagination {
  experiment_id: string;
  runs: RunSummary[];
}

export interface ExperimentSummary {
  experiment_id: string;
  status: ExperimentStatus;
  result_source: ResultSource;
  result: ExperimentResult | null;
  run_count: number;
  metrics: MetricSummary[];
}

export interface ExperimentExport {
  schema_version: 1;
  experiment: Experiment;
  summary: ExperimentSummary;
  runs: RunSummary[];
  batches: MetricBatch[];
}

export interface AppendMetricsResponse { status: 'accepted'; event_count: number }
export interface APIError { error: string }
