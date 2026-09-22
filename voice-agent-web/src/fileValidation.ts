export const MAX_WAV_BYTES = 2 * 1024 * 1024

export function validateWavFile(file: Pick<File, 'name' | 'size'>): string | null {
  if (!file.name.toLowerCase().endsWith('.wav')) return 'Please choose a WAV file with a .wav extension.'
  if (file.size > MAX_WAV_BYTES) return 'That WAV file is too large. Please choose a file no larger than 2 MB.'
  return null
}
