import type { createPhemeClient } from '../modules/pheme-native/src/client.ts';
import type { LocalResult } from '../modules/pheme-native/src/PhemeNative.types.ts';
import type { Readiness, Turn } from './types.ts';

type Client = ReturnType<typeof createPhemeClient>;
type Event = { event: string; data: Record<string, unknown> };
type Listener = (event: Event) => void;

// UI turn state only. Successful reasoning history and response validation stay
// in Pheme. No network transport or remote fallback exists in this adapter.
export function createLocalChat(client: Client, persist: (result: LocalResult) => void) {
  let current: Turn | null = null;
  let working = false;
  let cancelled = false;
  let sequence = 0;

  async function exclusive<T>(work: () => Promise<T>): Promise<T> {
    if (working) throw new Error('Wait for the active on-device operation to finish.');
    working = true;
    try { return await work(); } finally { working = false; }
  }

  function emit(listener: Listener, event: string, data: Record<string, unknown> = {}) {
    listener({ event, data: { turn_id: current?.id, ...data } });
  }

  async function reply(text: string, listener: Listener) {
    if (!current) throw new Error('No local turn to approve.');
    current = { ...current, status: 'generating', approvedText: text };
    emit(listener, 'question.approved', { text });
    const result = await client.respond(text, current.id);
    if (cancelled) throw new Error('Local turn cancelled.');
    if (result.execution_location !== 'device' || !result.reply.run_id || !result.reply.text.trim()) {
      throw new Error('Invalid on-device reply.');
    }
    current = { ...current, status: 'completed', reply: result.reply.text };
    emit(listener, 'reply.completed', { text: result.reply.text, run_id: result.reply.run_id });
  }

  async function run(work: () => Promise<void>, listener: Listener) {
    return exclusive(async () => {
      cancelled = false;
      try { await work(); }
      catch (error) {
        const message = error instanceof Error ? error.message : 'Local inference failed.';
        if (current) current = { ...current, status: cancelled ? 'cancelled' : 'failed', error: message };
        emit(listener, cancelled ? 'turn.cancelled' : 'turn.failed', { error: { message } });
        if (!cancelled) throw error;
      }
    });
  }

  return {
    async getReadiness(): Promise<Readiness> {
      return exclusive(async () => {
        let state = await client.open();
        if (!state.snapshot.ready || !state.model) {
          const models = await client.listModels();
          const saved = await client.savedSettings();
          const model = models.find(item => item.id === saved?.modelID) ?? models[0];
          if (!model) throw new Error('No ASR model. Bundle models with npm run native:bundle or import one in Model settings.');
          await client.configure(saved && saved.modelID === model.id ? saved :
            { modelID: model.id, language: 'en', dictionary: [], threads: 4, maxAudioSeconds: 120, metricsEnabled: true });
        }
        if (!state.reasoning?.ready || !state.reasoningModel) {
          const models = await client.listReasoningModels();
          if (!models[0]) throw new Error('No reasoning model. Rebuild this Expo app with npm run native:bundle.');
          await client.configureReasoning(models[0].id);
        }
        state = await client.inspect();
        return {
          stt: { name: state.model?.name ?? 'Whisper', ready: state.snapshot.ready && !!state.model },
          reply: { name: state.reasoningModel?.name ?? 'Reasoning', ready: state.reasoning?.ready === true && !!state.reasoningModel },
          maxAudioSeconds: state.settings?.maxAudioSeconds ?? 120,
        };
      });
    },
    async startTurn(input: string | { uri: string }, listener: Listener, signal?: AbortSignal) {
      if (signal?.aborted) throw new Error('Local turn cancelled.');
      if (working || current && ['transcribing', 'awaiting_review', 'generating'].includes(current.status)) {
        throw new Error('Finish or discard the current local turn first.');
      }
      current = { id: `local-${Date.now()}-${++sequence}`, status: 'transcribing', transcript: '',
        approvedText: null, reply: '', attachments: [] };
      const abort = () => { cancelled = true; void client.cancel(); };
      signal?.addEventListener('abort', abort, { once: true });
      try {
        await run(async () => {
          if (typeof input === 'string') { await reply(input, listener); return; }
          const result = await client.transcribeFile(input.uri);
          if (cancelled) throw new Error('Local turn cancelled.');
          if (result.execution_location !== 'device' || !result.transcription.run_id) throw new Error('Invalid local transcript.');
          persist(result);
          const text = result.transcription.result.text;
          if (!text.trim()) throw new Error('No speech detected. Record again.');
          current = { ...current!, status: 'awaiting_review', transcript: text };
          emit(listener, 'transcript.ready', { text });
        }, listener);
      } finally { signal?.removeEventListener('abort', abort); }
    },
    async submitTurn(id: string, text: string) {
      if (!current || current.id !== id || current.status !== 'awaiting_review' || !text.trim()) {
        throw new Error('This local transcript is no longer awaiting review.');
      }
      await run(() => reply(text.trim(), () => {}), () => {});
    },
    async getTurn(id: string): Promise<Turn> {
      if (!current || current.id !== id) throw new Error('This saved chat is read-only. Start a new local conversation.');
      return { ...current };
    },
    async cancelTurn(id: string) {
      if (current?.id !== id) return;
      cancelled = true;
      await client.cancel();
      if (!working) current = { ...current, status: 'cancelled' };
    },
    async resetChat() {
      await exclusive(async () => { await client.resetConversation(); current = null; });
    },
  };
}
