import type { DeviceInfo, Inspection, LocalReply, LocalResult, Model, NativeTransport, Observation, ReasoningSnapshot, Settings, Snapshot } from './PhemeNative.types.ts';

export class PhemeError extends Error {
  readonly code: string;
  readonly runID?: string;
  readonly attemptID?: string;
  readonly runtimeSessionID?: string;
  constructor(code: string, message: string, runID?: string, attemptID?: string, runtimeSessionID?: string) { super(message); this.name = 'PhemeError'; this.code = code; this.runID = runID; this.attemptID = attemptID; this.runtimeSessionID = runtimeSessionID; }
}

export function sameSettings(a: Settings, b: Settings | null): boolean {
  return b !== null && a.modelID === b.modelID && a.language === b.language && a.threads === b.threads &&
    a.maxAudioSeconds === b.maxAudioSeconds && a.metricsEnabled === b.metricsEnabled && JSON.stringify(a.dictionary) === JSON.stringify(b.dictionary);
}

function decode<T>(raw: string): T {
  let response: { ok?: boolean; value?: T; error?: { code?: string; message?: string; run_id?: string; attempt_id?: string; runtime_session_id?: string } };
  try { response = JSON.parse(raw); } catch { throw new PhemeError('invalid_response', 'Pheme returned malformed native JSON.'); }
  if (!response || typeof response !== 'object') throw new PhemeError('invalid_response', 'Pheme returned an invalid native response.');
  if (response.ok !== true) throw new PhemeError(response.error?.code ?? 'native_failure', response.error?.message ?? 'Pheme operation failed.', response.error?.run_id, response.error?.attempt_id, response.error?.runtime_session_id);
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
    deviceInfo: async () => {
      const transport = native();
      if (!transport.deviceInfo) throw new PhemeError('native_update_required', 'Rebuild and reinstall the native app to enable metrics sync.');
      return decode<DeviceInfo>(await transport.deviceInfo());
    },
    open: async () => decode<Inspection>(await native().open()),
    inspect: async () => decode<Inspection>(await native().inspect()),
    listModels: async () => decode<Model[]>(await native().listModels()),
    savedSettings: async () => decode<Settings | null>(await native().savedSettings()),
    importModel: async (uri: string, name: string, sha256: string) => decode<Model>(await native().importModel(uri, name, sha256)),
    configure: async (settings: Settings) => decode<Snapshot>(await native().configure(JSON.stringify(settings))),
    transcribeFile: async (uri: string, attemptID?: string) => {
      const transport = native();
      if (attemptID === undefined) return decode<LocalResult>(await transport.transcribeFile(uri));
      if (!transport.transcribeFileWithAttempt) throw new PhemeError('native_update_required', 'Rebuild and reinstall the native app for attributed collection.');
      return decode<LocalResult>(await transport.transcribeFileWithAttempt(uri, attemptID));
    },
    listReasoningModels: async () => decode<Model[]>(await native().listReasoningModels()),
    configureReasoning: async (modelID: string, threads = 4) => decode<ReasoningSnapshot>(await native().configureReasoning(modelID, threads)),
    respond: async (text: string, requestID: string, attemptID?: string) => {
      const transport = native();
      if (attemptID === undefined) return decode<LocalReply>(await transport.respond(text, requestID));
      if (!transport.respondWithAttempt) throw new PhemeError('native_update_required', 'Rebuild and reinstall the native app for attributed collection.');
      return decode<LocalReply>(await transport.respondWithAttempt(text, requestID, attemptID));
    },
    setBenchmarkActive: async (active: boolean) => {
      const transport = native();
      if (!transport.setBenchmarkActive) throw new PhemeError('native_update_required', 'Rebuild and reinstall the native app for benchmark display control.');
      decode<null>(await transport.setBenchmarkActive(active));
    },
    setResourceSamplingEnabled: async (enabled: boolean) => {
      const transport = native();
      if (!transport.setResourceSamplingEnabled) throw new PhemeError('native_update_required', 'Rebuild and reinstall the native app for sampler configuration.');
      decode<null>(await transport.setResourceSamplingEnabled(enabled));
    },
    resetConversation: async () => decode<ReasoningSnapshot>(await native().resetConversation()),
    poll: async () => decode<Observation>(await native().poll()),
    cancel: async () => native().cancel(),
    close: async () => { if (transport) await transport.close(); },
  };
}
