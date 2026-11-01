const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "/api/v1"

export type ExperimentStatus = "queued" | "running" | "completed" | "failed"
export type MetricValue = number | string | boolean | null
export interface Experiment { id: string; result_source: string; use_case: string; dataset: string; model_config: { name?: string; version?: string; quantization?: string; runtime?: string }; environment_profile: { name?: string; type?: string; cpu_cores?: number; memory_mb?: number; gpu_info?: string }; status: ExperimentStatus; created_at: string; started_at?: string; completed_at?: string; result?: Record<string, unknown> }
export interface ExperimentPage { experiments: Experiment[]; total: number; limit: number; offset: number }
export interface MetricSummary { name: string; unit: string; scope: string; source: string; observation_count: number; numeric_count: number; unavailable_count: number; unavailable_reasons: string[]; min: number | null; max: number | null; mean: number | null }
export interface Summary { experiment_id: string; status: ExperimentStatus; result_source: string; result: Record<string, unknown> | null; run_count: number; metrics: MetricSummary[] }
export interface MetricBatch { schema_version: number; experiment_id: string; run_id: string; sequence_number: number; timestamp_ms: number; events: MetricEvent[] }
export interface MetricEvent { schema_version?: number; run_id?: string; sequence?: number; name: string; unit: string; scope: string; source: string; value: MetricValue; timestamp_ms: number; unavailable_reason?: string }
export interface MetricsPage { experiment_id: string; batches: MetricBatch[]; total: number; limit: number; offset: number }
export interface RunSummary { run_id: string; batch_count: number; event_count: number; first_timestamp_ms: number; last_timestamp_ms: number; metrics: MetricSummary[] }
export interface RunPage { experiment_id: string; runs: RunSummary[]; total: number; limit: number; offset: number }
export interface ExportData { schema_version: number; experiment: Experiment; summary: Summary; runs: unknown[]; batches: MetricBatch[] }
export interface VoiceMetricsPage { source: "phone_voice"; batches: MetricBatch[]; total: number }
export interface VoiceSession { id: string; status: string; transcript: string; approved_text?: string; reply: string; error?: string; created_at: string; updated_at: string }
export interface VoiceSessionsPage { sessions: VoiceSession[]; total: number }

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`, { ...init, headers: { "Content-Type": "application/json", ...(init?.headers ?? {}) } })
  if (!response.ok) { const body = await response.json().catch(() => ({})) as { error?: string }; throw new Error(body.error ?? `Request failed (${response.status})`) }
  return response.json() as Promise<T>
}
export const listExperiments = () => request<ExperimentPage>("/experiments?limit=100")
export const getExperiment = (id: string) => request<Experiment>(`/experiments/${encodeURIComponent(id)}`)
export const getSummary = (id: string) => request<Summary>(`/experiments/${encodeURIComponent(id)}/summary`)
export const getMetrics = (id: string) => request<MetricsPage>(`/experiments/${encodeURIComponent(id)}/metrics?limit=100`)
export const getRuns = (id: string) => request<RunPage>(`/experiments/${encodeURIComponent(id)}/runs?limit=100`)
export const getVoiceMetrics = () => request<VoiceMetricsPage>("/voice/metrics")
export const getVoiceSessions = () => request<VoiceSessionsPage>("/voice/sessions")
export const exportExperiment = (id: string) => request<ExportData>(`/experiments/${encodeURIComponent(id)}/export?format=json`)
export async function createExperiment(input: { use_case: string; dataset: string; cases: { id?: string; transcript: string }[]; model: string; quantization: string; runtime: string; profile: string; cpu_cores: number; memory_mb: number }) {
  return request<{ id: string; status: ExperimentStatus }>("/experiments", { method: "POST", body: JSON.stringify({ use_case: input.use_case, dataset: input.dataset, cases: input.cases, model_config: { name: input.model, quantization: input.quantization, runtime: input.runtime }, environment_profile: { name: input.profile, type: "cpu", cpu_cores: input.cpu_cores, memory_mb: input.memory_mb } }) })
}
