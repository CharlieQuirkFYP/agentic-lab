import assert from 'node:assert/strict';
import test from 'node:test';
import { createPhemeClient, PhemeError, sameSettings } from '../modules/pheme-native/src/client.ts';
import type { NativeTransport, Settings } from '../modules/pheme-native/src/PhemeNative.types.ts';

const ok = (value: unknown) => JSON.stringify({ ok: true, value });
function fake(overrides: Partial<NativeTransport> = {}): NativeTransport {
  return { open: async () => ok({ busy: false }), inspect: async () => ok({ busy: false }), listModels: async () => ok([]), savedSettings: async () => ok(null),
    importModel: async () => ok({ id: 'artifact' }), configure: async () => ok({ ready: true }),
    transcribeFile: async () => ok({ transcription: { run_id: 'run-1' }, execution_location: 'device' }),
    poll: async () => ok({ batchesJSON: '[]' }), cancel: async () => {}, close: async () => {}, ...overrides };
}

test('missing native build fails explicitly without making network requests', async () => {
  const client = createPhemeClient(null);
  assert.equal(client.available, false);
  await assert.rejects(client.inspect(), (e: unknown) => e instanceof PhemeError && e.code === 'native_unavailable');
  await client.close();
});

test('passes file URI and settings directly; retains run ID and exact metric payload', async () => {
  let calledURI = ''; let calledSettings = '';
  const batchesJSON = '[ { "run_id": "run-1", "sequence_number": 0 } ]';
  const client = createPhemeClient(fake({
    transcribeFile: async uri => { calledURI = uri; return ok({ transcription: { run_id: 'run-1' }, execution_location: 'device' }); },
    configure: async settings => { calledSettings = settings; return ok({ ready: true }); },
    poll: async () => ok({ batchesJSON }),
  }));
  const settings: Settings = { modelID: 'hash', language: 'en', dictionary: ['KLASS'], threads: 4, maxAudioSeconds: 120, metricsEnabled: true };
  await client.configure(settings);
  assert.deepEqual(JSON.parse(calledSettings), settings);
  assert.equal((await client.transcribeFile('file:///app/speech.wav')).transcription.run_id, 'run-1');
  assert.equal(calledURI, 'file:///app/speech.wav');
  assert.equal((await client.poll()).batchesJSON, batchesJSON);
});

test('native failures preserve stable code and admitted run ID', async () => {
  const client = createPhemeClient(fake({ transcribeFile: async () => JSON.stringify({ ok: false, error: { code: 'native_2', message: 'Inference failed', run_id: 'failed-1' } }) }));
  await assert.rejects(client.transcribeFile('file:///app/test.wav'), (e: unknown) => e instanceof PhemeError && e.code === 'native_2' && e.runID === 'failed-1');
});

test('malformed and missing native results do not appear successful', async () => {
  for (const raw of ['invalid', 'null', '[]', '{"ok":true}']) {
    await assert.rejects(createPhemeClient(fake({ inspect: async () => raw })).inspect(), PhemeError);
  }
});

test('cancel and close use native lifecycle operations', async () => {
  let cancelled = 0; let closed = 0;
  const client = createPhemeClient(fake({ cancel: async () => { cancelled++; }, close: async () => { closed++; } }));
  await client.cancel(); await client.close();
  assert.equal(cancelled, 1); assert.equal(closed, 1);
});

test('recording readiness requires applied native settings rather than an unsubmitted selection', () => {
  const settings: Settings = { modelID: 'hash', language: 'en', dictionary: ['KLASS'], threads: 4, maxAudioSeconds: 30, metricsEnabled: true };
  assert.equal(sameSettings(settings, null), false);
  assert.equal(sameSettings(settings, { ...settings }), true);
  for (const change of [{ modelID: 'other' }, { language: null }, { dictionary: ['gate'] }, { threads: 2 }, { maxAudioSeconds: 120 }, { metricsEnabled: false }]) {
    assert.equal(sameSettings(settings, { ...settings, ...change }), false);
  }
});
