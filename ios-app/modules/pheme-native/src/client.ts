import type { Inspection, LocalResult, Model, NativeTransport, Observation, Settings, Snapshot } from './PhemeNative.types.ts';

export class PhemeError extends Error {
  readonly code: string;
  readonly runID?: string;
  constructor(code: string, message: string, runID?: string) { super(message); this.name = 'PhemeError'; this.code = code; this.runID = runID; }
}

export function sameSettings(a: Settings, b: Settings | null): boolean {
  return b !== null && a.modelID === b.modelID && a.language === b.language && a.threads === b.threads &&
    a.maxAudioSeconds === b.maxAudioSeconds && a.metricsEnabled === b.metricsEnabled && JSON.stringify(a.dictionary) === JSON.stringify(b.dictionary);
}

function decode<T>(raw: string): T {
  let response: { ok?: boolean; value?: T; error?: { code?: string; message?: string; run_id?: string } };
  try { response = JSON.parse(raw); } catch { throw new PhemeError('invalid_response', 'Pheme returned malformed native JSON.'); }
  if (!response || typeof response !== 'object') throw new PhemeError('invalid_response', 'Pheme returned an invalid native response.');
  if (response.ok !== true) throw new PhemeError(response.error?.code ?? 'native_failure', response.error?.message ?? 'Pheme operation failed.', response.error?.run_id);
  if (!Object.hasOwn(response, 'value')) throw new PhemeError('invalid_response', 'Pheme returned no result.');
  return response.value as T;
}

export function createPhemeClient(transport: NativeTransport | null) {
  function native(): NativeTransport {
    if (!transport) throw new PhemeError('native_unavailable', 'Install the iOS Pheme development build. Expo Go does not contain this module.');
    return transport;
  }
  return {
    available: transport !== null,
    open: async () => decode<Inspection>(await native().open()),
    inspect: async () => decode<Inspection>(await native().inspect()),
    listModels: async () => decode<Model[]>(await native().listModels()),
    savedSettings: async () => decode<Settings | null>(await native().savedSettings()),
    importModel: async (uri: string, name: string, sha256: string) => decode<Model>(await native().importModel(uri, name, sha256)),
    configure: async (settings: Settings) => decode<Snapshot>(await native().configure(JSON.stringify(settings))),
    transcribeFile: async (uri: string) => decode<LocalResult>(await native().transcribeFile(uri)),
    poll: async () => decode<Observation>(await native().poll()),
    cancel: async () => native().cancel(),
    close: async () => { if (transport) await transport.close(); },
  };
}
