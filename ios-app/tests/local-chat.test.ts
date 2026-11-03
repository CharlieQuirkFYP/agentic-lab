import assert from 'node:assert/strict';
import test from 'node:test';
import { createPhemeClient } from '../modules/pheme-native/src/client.ts';
import type { NativeTransport } from '../modules/pheme-native/src/PhemeNative.types.ts';
import { createLocalChat } from '../src/localChat.ts';

const ok = (value: unknown) => JSON.stringify({ ok: true, value });
function fixture(overrides: Partial<NativeTransport> = {}) {
  const calls: string[] = [];
  const model = { id: 'hash', sha256: 'hash', name: 'test model', bytes: 3 };
  let asr = false, reasoning = false;
  const inspect = () => ok({ snapshot: { ready: asr }, reasoning: { ready: reasoning }, model: asr ? model : null, reasoningModel: reasoning ? model : null });
  const transport: NativeTransport = {
    open: async () => { calls.push('open'); return inspect(); },
    inspect: async () => inspect(), listModels: async () => ok([model]), savedSettings: async () => ok(null),
    listReasoningModels: async () => ok([model]), importModel: async () => ok(model),
    configure: async () => { asr = true; calls.push('asr'); return ok({ ready: true }); },
    configureReasoning: async () => { reasoning = true; calls.push('reasoning'); return ok({ ready: true }); },
    transcribeFile: async uri => { calls.push(uri); return ok({ execution_location: 'device', transcription: { run_id: 't1', result: { text: 'Smoke at entrance' } }, model }); },
    respond: async (text, id) => { calls.push(`reply:${text}:${id}`); return ok({ execution_location: 'device', reply: { run_id: 'r1', text: 'Which entrance?' }, model }); },
    resetConversation: async () => { calls.push('reset'); return ok({ ready: true }); },
    cancel: async () => { calls.push('cancel'); }, close: async () => {}, poll: async () => ok({}),
    ...overrides,
  };
  const chat = createLocalChat(createPhemeClient(transport), () => calls.push('persist'));
  return { chat, calls };
}

test('loads both local models once and reuses warm native engines', async () => {
  const { chat, calls } = fixture();
  assert.equal((await chat.getReadiness()).reply.ready, true);
  await chat.getReadiness();
  assert.deepEqual(calls, ['open', 'asr', 'reasoning', 'open']);
});

test('audio is persisted and reviewed before reasoning; reset calls Pheme', async () => {
  const { chat, calls } = fixture();
  const events: string[] = [];
  let id = '';
  await chat.startTurn({ uri: 'file:///app/audio.wav' }, event => { events.push(event.event); id = String(event.data.turn_id); });
  assert.deepEqual(events, ['transcript.ready']);
  assert.deepEqual(calls, ['file:///app/audio.wav', 'persist']);
  assert.equal((await chat.getTurn(id)).status, 'awaiting_review');
  await assert.rejects(chat.startTurn('Overlap', () => {}), /current local turn/);
  await chat.submitTurn(id, 'Smoke at west entrance');
  assert.equal((await chat.getTurn(id)).reply, 'Which entrance?');
  assert.ok(calls.includes(`reply:Smoke at west entrance:${id}`));
  await assert.rejects(chat.submitTurn(id, 'Duplicate'), /no longer awaiting/);
  await chat.resetChat();
  assert.equal(calls.at(-1), 'reset');
  await assert.rejects(chat.getTurn(id), /read-only/);
});

test('typed messages invoke native reasoning directly, without an HTTP service', async () => {
  const { chat, calls } = fixture();
  const events: string[] = [];
  await chat.startTurn('Smoke near gate', event => events.push(event.event));
  assert.deepEqual(events, ['question.approved', 'reply.completed']);
  assert.match(calls[0], /^reply:Smoke near gate:local-/);
});

test('cancel keeps admission closed until native work settles and discards late output', async () => {
  let release!: (value: string) => void;
  const { chat, calls } = fixture({ respond: () => new Promise(resolve => { release = resolve; }) });
  let id = '';
  const events: string[] = [];
  const turn = chat.startTurn('Smoke', event => { id = String(event.data.turn_id); events.push(event.event); });
  await chat.cancelTurn(id);
  await assert.rejects(chat.resetChat(), /active on-device/);
  await assert.rejects(chat.startTurn('Overlap', () => {}), /current local turn/);
  release(ok({ execution_location: 'device', reply: { text: 'Discard this', run_id: 'r2' } }));
  await turn;
  assert.equal((await chat.getTurn(id)).status, 'cancelled');
  assert.deepEqual(events, ['question.approved', 'turn.cancelled']);
  assert.deepEqual(calls, ['cancel']);
});

test('missing native build or model fails explicitly instead of using desktop inference', async () => {
  await assert.rejects(createLocalChat(createPhemeClient(null), () => {}).getReadiness(), /Expo Go/);
  const { chat, calls } = fixture({ listReasoningModels: async () => ok([]) });
  await assert.rejects(chat.getReadiness(), /No reasoning model/);
  assert.deepEqual(calls, ['open', 'asr']);
});
