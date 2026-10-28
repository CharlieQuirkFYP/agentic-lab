export interface Model { id: string; name: string; sha256: string; bytes: number }
export interface Settings { modelID: string; language: string | null; dictionary: string[]; threads: number; maxAudioSeconds: number; metricsEnabled: boolean }
export interface Snapshot { schema_version: 1; state: string; ready: boolean; capabilities: { whisper: boolean; whisper_metal: boolean; whisper_coreml: boolean; metrics_output: boolean }; active_run?: string; configuration?: { max_audio_seconds: number; threads: number; model_path: string }; last_failure?: { code: number; message: string; run_id?: string } }
export interface Inspection { snapshot: Snapshot; busy: boolean; closed: boolean; model: Model | null; settings: Settings | null }
export interface Observation { snapshot: Snapshot; observer: { capacity_events: number; pending_events: number; dropped_events: number }; batchesJSON: string }
export interface LocalResult { transcription: { schema_version: 1; run_id: string; result: { text: string; status: string; model_id: string; processing_time_ms: number } }; model: Model; settings: Settings; execution_location: 'device' }
export interface NativeTransport {
  open(): Promise<string>;
  inspect(): Promise<string>;
  listModels(): Promise<string>;
  savedSettings(): Promise<string>;
  importModel(uri: string, name: string, sha256: string): Promise<string>;
  configure(settings: string): Promise<string>;
  transcribeFile(uri: string): Promise<string>;
  poll(): Promise<string>;
  cancel(): Promise<void>;
  close(): Promise<void>;
}
