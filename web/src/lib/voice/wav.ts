export const MAX_RECORDING_SECONDS = 60
export const MAX_WAV_BYTES = 16 * 1024 * 1024
export const MAX_TEXT_LENGTH = 4_000
export const MAX_REPLY_LENGTH = 64_000

export function validateText(text: string): string | null {
  if (!text.trim()) return 'Enter a non-empty question.'
  if (text.length > MAX_TEXT_LENGTH) return `Questions are limited to ${MAX_TEXT_LENGTH.toLocaleString()} characters.`
  return null
}

// Keep the AudioContext sample rate. Pheme owns authoritative resampling.
export function encodeWav(chunks: readonly Float32Array[], sampleRate: number): ArrayBuffer {
  if (!Number.isInteger(sampleRate) || sampleRate < 8_000 || sampleRate > 192_000) {
    throw new Error('Unsupported microphone sample rate.')
  }
  const samples = chunks.reduce((sum, chunk) => sum + chunk.length, 0)
  if (!samples) throw new Error('No microphone audio was captured. Record again or use text input.')
  if (samples > sampleRate * MAX_RECORDING_SECONDS) throw new Error('Recording exceeds the duration limit.')
  const dataBytes = samples * 2
  if (44 + dataBytes > MAX_WAV_BYTES) throw new Error('Recording exceeds the upload size limit.')
  const buffer = new ArrayBuffer(44 + dataBytes)
  const view = new DataView(buffer)
  const ascii = (offset: number, text: string) => {
    for (let i = 0; i < text.length; i++) view.setUint8(offset + i, text.charCodeAt(i))
  }
  ascii(0, 'RIFF')
  view.setUint32(4, 36 + dataBytes, true)
  ascii(8, 'WAVE')
  ascii(12, 'fmt ')
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true) // Linear PCM
  view.setUint16(22, 1, true) // Mono
  view.setUint32(24, sampleRate, true)
  view.setUint32(28, sampleRate * 2, true)
  view.setUint16(32, 2, true)
  view.setUint16(34, 16, true)
  ascii(36, 'data')
  view.setUint32(40, dataBytes, true)
  let offset = 44
  for (const chunk of chunks) {
    for (const value of chunk) {
      const sample = Number.isFinite(value) ? Math.max(-1, Math.min(1, value)) : 0
      view.setInt16(offset, Math.round(sample * (sample < 0 ? 32768 : 32767)), true)
      offset += 2
    }
  }
  return buffer
}
