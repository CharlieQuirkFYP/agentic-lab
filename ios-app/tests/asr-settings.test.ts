import assert from 'node:assert/strict';
import test from 'node:test';
import { parseVocabularyHints } from '../src/asrSettings.ts';

test('comma-separated ASR hints retain verified spelling, phrases and order', () => {
  assert.deepEqual(parseVocabularyHints(' KLASS, Bukit Panjang , , Jalan Buroh,  '),
    ['KLASS', 'Bukit Panjang', 'Jalan Buroh']);
  assert.deepEqual(parseVocabularyHints('  , ,\n'), []);
  assert.deepEqual(parseVocabularyHints(''), []);
  assert.deepEqual(parseVocabularyHints('D’Silva, Block A / Level 2'), ['D’Silva', 'Block A / Level 2']);
});

test('typing a separator preserves the previous hint until the next is entered', () => {
  assert.deepEqual(parseVocabularyHints('KLASS,'), ['KLASS']);
  assert.deepEqual(parseVocabularyHints('KLASS, '), ['KLASS']);
  assert.deepEqual(parseVocabularyHints('KLASS, west entrance'), ['KLASS', 'west entrance']);
});
