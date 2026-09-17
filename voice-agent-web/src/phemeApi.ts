export type Transcription = {
  status: string
  text: string
  language: string | null
  processing_time_ms: number
}

export type IncidentReport = {
  incident_type: string
  location: string
  severity: string
  summary: string
  recommended_action: string
}

type ApiError = { error?: { message?: string } }

async function request<T>(path: string, options: RequestInit): Promise<T> {
  const response = await fetch(`/pheme${path}`, options)
  if (!response.ok) {
    const body = await response.json().catch(() => ({})) as ApiError
    throw new Error(body.error?.message ?? `The voice service returned ${response.status}.`)
  }
  return response.json() as Promise<T>
}

export function transcribe(audio: Blob): Promise<Transcription> {
  return request('/v1/transcribe', { method: 'POST', headers: { 'Content-Type': 'audio/wav' }, body: audio })
}

export function analyze(transcript: string): Promise<IncidentReport> {
  return request('/v1/analyze', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ transcript }) })
}
