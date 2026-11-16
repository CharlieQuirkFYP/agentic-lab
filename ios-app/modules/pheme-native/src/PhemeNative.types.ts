export interface Model { id: string; name: string; sha256: string; bytes: number }
export interface Settings { modelID: string; language: string | null; dictionary: string[]; threads: number; maxAudioSeconds: number; metricsEnabled: boolean }
export interface Snapshot { schema_version: 1; state: string; ready: boolean; capabilities: { whisper: boolean; whisper_metal: boolean; whisper_coreml: boolean; metrics_output: boolean }; active_run?: string; configuration?: { max_audio_seconds: number; threads: number; model_path: string }; last_failure?: { code: number; message: string; run_id?: string } }
export interface ReasoningEffectiveSettings { max_input_chars: number; max_output_chars: number; max_history_turns: number; max_output_tokens: number; context_size: number; temperature: number; top_k: number; top_p: number; min_p: number; seed: number | null }
export interface ReasoningSnapshot { ready: boolean; state: string; model_path?: string; threads?: number; active_run?: string; history_turns: number; load_time_ms: number; conversation_config?: ReasoningEffectiveSettings; gpu_layers?: number; runtime?: { name: string; version: string } }
export interface NativeAttempt { attempt_id: string; native_run_id: string; stage: 'transcription' | 'reasoning' }
export interface NativeResourceValue { name: string; unit: string; scope: string; source: string; value: number | string | null; unavailable_reason?: string }
export interface NativeResourceSample { runtime_session_id: string; attempt_id?: string; native_run_id?: string; stage: 'transcription' | 'reasoning' | 'idle'; timestamp_ms: number; monotonic_ms: number; interval_ms: number | null; boundary: 'start' | 'periodic' | 'finish'; values: NativeResourceValue[] }
export interface NativeResourceObserver { capacity_samples: number; pending_samples: number; dropped_samples: number; interval_ms: number; enabled?: boolean }
export interface EffectiveConfiguration {
  schema_version: 1;
  asr: { model: Model; settings: Settings; dictionary_sha256: string; capabilities: Snapshot['capabilities']; runtime: string } | null;
  reasoning: { model: Model; threads: number | null; prompt_sha256: string | null; settings: ReasoningEffectiveSettings | null; gpu_layers: number | null; runtime: { name: string; version: string } | null } | null;
  sampler: { enabled: boolean; interval_ms: number; capacity_samples: number; cpu_percent_basis: 'one_core'; scope: 'whole_app_process'; cpu_min_interval_ms: number };
  app_version: string; app_build: string;
}
export interface LocalReply { reply: { run_id: string; text: string; processing_time_ms: number }; model: Model; execution_location: 'device'; runtime_session_id?: string; attempt_id?: string | null }
export interface Inspection { snapshot: Snapshot; busy: boolean; closed: boolean; model: Model | null; settings: Settings | null; reasoning: ReasoningSnapshot | null; reasoningModel: Model | null; runtime_session_id?: string; active_attempt?: NativeAttempt | null; effective_configuration?: EffectiveConfiguration; benchmark_active?: boolean; sampler?: NativeResourceObserver }
export interface Observation { snapshot: Snapshot; observer: { capacity_events: number; pending_events: number; dropped_events: number }; batchesJSON: string; runtime_session_id?: string; active_attempt?: NativeAttempt | null; resource_samples?: NativeResourceSample[]; resource_observer?: NativeResourceObserver }
export interface LocalResult { transcription: { schema_version: 1; run_id: string; result: { text: string; status: string; model_id: string; processing_time_ms: number } }; model: Model; settings: Settings; execution_location: 'device'; runtime_session_id?: string; attempt_id?: string | null }
export interface DeviceInfo { hardware: string; os_version: string; app_version: string; build: string }
export interface NativeTransport {
  deviceInfo?(): Promise<string>;
  open(): Promise<string>;
  inspect(): Promise<string>;
  listModels(): Promise<string>;
  savedSettings(): Promise<string>;
  importModel(uri: string, name: string, sha256: string): Promise<string>;
  configure(settings: string): Promise<string>;
  transcribeFile(uri: string): Promise<string>;
  transcribeFileWithAttempt?(uri: string, attemptID: string): Promise<string>;
  listReasoningModels(): Promise<string>;
  configureReasoning(modelID: string, threads: number): Promise<string>;
  respond(text: string, requestID: string): Promise<string>;
  respondWithAttempt?(text: string, requestID: string, attemptID: string): Promise<string>;
  setBenchmarkActive?(active: boolean): Promise<string>;
  setResourceSamplingEnabled?(enabled: boolean): Promise<string>;
  resetConversation(): Promise<string>;
  poll(): Promise<string>;
  cancel(): Promise<void>;
  close(): Promise<void>;
}
