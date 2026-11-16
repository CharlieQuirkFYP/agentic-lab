import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { defaultModelSelection, parseModelOptions, selectedModels } from '../scripts/model-bundle.mjs';

test('bundle model arrays retain defaults, persist previous choices and accept explicit CLI/environment IDs', () => {
  assert.deepEqual(parseModelOptions([], {}).selection, defaultModelSelection);
  const selection = { transcription: ['whisper-tiny.en', 'whisper-large-v3-turbo'], reasoning: ['qwen2.5-1.5b-instruct-q4-k-m'] };
  assert.deepEqual(parseModelOptions([], {}, selection).selection, selection);
  assert.deepEqual(parseModelOptions(['--asr-models', selection.transcription.join(',')], { PHEME_IOS_REASONING_MODELS: selection.reasoning[0] }).selection, selection);
  assert.deepEqual(parseModelOptions(['--asr-models', 'whisper-tiny.en'], { PHEME_IOS_ASR_MODELS: 'whisper-large-v3-turbo' }).selection.transcription, ['whisper-tiny.en']);
  for (const value of ['', '../model.bin', '/tmp/model.bin', 'whisper;touch', 'model,,other', 'same,same', ' model', 'a'.repeat(97), Array.from({ length: 9 }, (_, i) => `m${i}`).join(',')]) {
    assert.throws(() => parseModelOptions(['--asr-models', value], {}));
  }
  for (const args of [['--unknown'], ['--asr-models'], ['--asr-models', '--check'], ['--without-models', '--asr-models', 'tiny'], ['--bundle-models', '--without-models'], ['--check', '--check']]) {
    assert.throws(() => parseModelOptions(args, {}));
  }
});

test('phone model choices require pinned, compatible manifest artifacts and intact license resources', () => {
  const root = resolve(import.meta.dirname, '../../pheme-va');
  const selection = { transcription: ['whisper-large-v3-turbo', 'whisper-tiny.en'], reasoning: ['qwen3-4b-instruct-2507-q4-k-m', 'qwen2.5-1.5b-instruct-q4-k-m'] };
  const models = selectedModels(root, selection);
  assert.equal(models.length, 4);
  assert.equal(models.reduce((sum, model) => sum + model.model_size_bytes, 0), 5316861846);
  assert.throws(() => selectedModels(root, { ...selection, transcription: ['zipformer-small'] }), /Unsupported/);
  assert.throws(() => selectedModels(root, { ...selection, reasoning: ['whisper-tiny.en'] }), /Unsupported/);
  assert.throws(() => selectedModels(root, { ...selection, reasoning: ['missing'] }), /Missing pinned/);

  const fixture = mkdtempSync(join(tmpdir(), 'model-selection-'));
  try {
    mkdirSync(join(fixture, 'models/licenses'), { recursive: true });
    for (const name of ['whisper-MIT.txt', 'qwen-Apache-2.0.txt']) writeFileSync(join(fixture, 'models/licenses', name), readFileSync(join(root, 'models/licenses', name)));
    const manifest = readFileSync(join(root, 'models/manifest.toml'), 'utf8');
    for (const [before, after, error] of [
      ['transcript/whisper/ggml-tiny.en.bin', '../outside.bin', /Invalid manifest artifact/],
      ['license = "mit"', 'license = "unknown"', /Missing verified model license/],
      ['model_size_bytes = 77704715', 'model_size_bytes = 0', /Incomplete pinned/],
    ]) {
      writeFileSync(join(fixture, 'models/manifest.toml'), manifest.replace(before, after));
      assert.throws(() => selectedModels(fixture, selection), error);
    }
  } finally { rmSync(fixture, { recursive: true, force: true }); }
});
