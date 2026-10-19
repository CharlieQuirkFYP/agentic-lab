import assert from 'node:assert/strict'
import test from 'node:test'

import { encodeWav, MAX_RECORDING_SECONDS, MAX_TEXT_LENGTH, validateText } from '../src/lib/voice/wav.ts'

function ascii(view: DataView, offset: number, length: number) {
  return String.fromCharCode(...Array.from({ length }, (_, index) => view.getUint8(offset + index)))
}

test('WAV is a complete mono PCM16 RIFF file with native sample rate', () => {
  const view = new DataView(encodeWav([new Float32Array([-1, -0.5]), new Float32Array([0, 0.5, 1])], 48_000))
  assert.equal(view.byteLength, 54)
  assert.equal(ascii(view, 0, 4), 'RIFF')
  assert.equal(view.getUint32(4, true), 46)
  assert.equal(ascii(view, 8, 4), 'WAVE')
  assert.equal(ascii(view, 12, 4), 'fmt ')
  assert.equal(view.getUint32(16, true), 16)
  assert.equal(view.getUint16(20, true), 1)
  assert.equal(view.getUint16(22, true), 1)
  assert.equal(view.getUint32(24, true), 48_000)
  assert.equal(view.getUint32(28, true), 96_000)
  assert.equal(view.getUint16(32, true), 2)
  assert.equal(view.getUint16(34, true), 16)
  assert.equal(ascii(view, 36, 4), 'data')
  assert.equal(view.getUint32(40, true), 10)
  assert.deepEqual(Array.from({ length: 5 }, (_, index) => view.getInt16(44 + index * 2, true)), [-32768, -16384, 0, 16384, 32767])
})

test('PCM clamps over-range/nonfinite samples safely and preserves silence', () => {
  const view = new DataView(encodeWav([new Float32Array([-2, 2, NaN, Infinity, 0])], 16_000))
  assert.deepEqual(Array.from({ length: 5 }, (_, index) => view.getInt16(44 + index * 2, true)), [-32768, 32767, 0, 0, 0])
})

test('WAV rejects empty, unsupported rate, over-duration, and over-size inputs', () => {
  assert.throws(() => encodeWav([], 16_000), /No microphone audio/)
  assert.throws(() => encodeWav([new Float32Array(1)], 0), /sample rate/)
  assert.throws(() => encodeWav([new Float32Array(1)], 16_000.5), /sample rate/)
  assert.throws(() => encodeWav([new Float32Array(8_000 * MAX_RECORDING_SECONDS + 1)], 8_000), /duration limit/)
  assert.throws(() => encodeWav([new Float32Array(192_000 * MAX_RECORDING_SECONDS)], 192_000), /size limit/)
})

test('text limits reject whitespace and preserve exact edited wording', () => {
  assert.match(validateText(' \n\t')!, /non-empty/)
  assert.equal(validateText('  Café 👋  '), null)
  assert.equal(validateText('a'.repeat(MAX_TEXT_LENGTH)), null)
  assert.match(validateText('a'.repeat(MAX_TEXT_LENGTH + 1))!, /limited/)
})
