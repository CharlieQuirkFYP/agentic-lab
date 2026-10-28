import assert from 'node:assert/strict';
import test from 'node:test';
import { transcribeLocally } from '../src/localWorkflow.ts';
import type { LocalResult } from '../modules/pheme-native/src/PhemeNative.types.ts';

const result = { execution_location: 'device', transcription: { run_id: 'local-1' } } as LocalResult;
test('local inference persists the result before removing its recovery file', async () => {
  const calls: string[] = [];
  await transcribeLocally('file:///recording.wav', {
    transcribeFile: async uri => { calls.push(uri); return result; },
    persist: () => { calls.push('persist'); }, releaseRecording: () => { calls.push('remove'); },
  });
  assert.deepEqual(calls, ['file:///recording.wav', 'persist', 'remove']);
});
test('failure, discarded result or storage error retains the recoverable recording', async () => {
  for (const failAt of ['inference', 'persistence', 'validation']) {
    let removed = false;
    await assert.rejects(transcribeLocally('file:///recording.wav', {
      transcribeFile: async () => { if (failAt === 'inference') throw new Error('cancelled'); return failAt === 'validation' ? { ...result, execution_location: 'remote' } as unknown as LocalResult : result; },
      persist: () => { if (failAt === 'persistence') throw new Error('disk full'); },
      releaseRecording: () => { removed = true; },
    }));
    assert.equal(removed, false);
  }
});
