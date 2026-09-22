import { describe, expect, it } from 'vitest'

import { MAX_WAV_BYTES, validateWavFile } from './fileValidation'

describe('validateWavFile', () => {
  it('accepts a WAV file at the size limit', () => {
    expect(validateWavFile({ name: 'recording.wav', size: MAX_WAV_BYTES })).toBeNull()
  })

  it('accepts uppercase WAV extensions', () => {
    expect(validateWavFile({ name: 'RECORDING.WAV', size: 100 })).toBeNull()
  })

  it('rejects non-WAV files', () => {
    expect(validateWavFile({ name: 'recording.mp3', size: 100 })).toContain('WAV')
  })

  it('rejects files above the 2 MiB limit', () => {
    expect(validateWavFile({ name: 'recording.wav', size: MAX_WAV_BYTES + 1 })).toContain('2 MB')
  })
})
