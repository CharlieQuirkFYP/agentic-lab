export function localVoices<T extends { localService: boolean }>(voices: readonly T[]): T[] {
  return voices.filter((voice) => voice.localService)
}

export function canAutoSpeak(liveCompletion: boolean, enabled: boolean, text: string): boolean {
  return liveCompletion && enabled && text.trim().length > 0
}
