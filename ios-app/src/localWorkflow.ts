import type { LocalResult } from '../modules/pheme-native/src/PhemeNative.types.ts';

export interface LocalWorkflowDependencies {
  transcribeFile(uri: string): Promise<LocalResult>;
  persist(result: LocalResult): void;
  releaseRecording(): void;
}

// Persist before removing the recovery recording. Tests inject local fakes;
// there is intentionally no HTTP fallback in this workflow.
export async function transcribeLocally(uri: string, dependencies: LocalWorkflowDependencies): Promise<LocalResult> {
  const result = await dependencies.transcribeFile(uri);
  if (result.execution_location !== 'device' || !result.transcription.run_id) throw new Error('Invalid local inference result.');
  dependencies.persist(result);
  dependencies.releaseRecording();
  return result;
}
