import { useEffect, useRef, useState } from 'react'

import { MicrophoneRecorder } from './audio'
import { analyze, transcribe, type IncidentReport } from './phemeApi'

type VoiceState = 'ready' | 'recording' | 'processing' | 'complete' | 'error'

const labels: Record<VoiceState, string> = {
  ready: 'Ready to listen', recording: 'Listening…', processing: 'Processing your report…', complete: 'Report ready', error: 'Something went wrong',
}

export default function App() {
  const recorder = useRef<MicrophoneRecorder | null>(null)
  const [state, setState] = useState<VoiceState>('ready')
  const [transcript, setTranscript] = useState('')
  const [report, setReport] = useState<IncidentReport | null>(null)
  const [error, setError] = useState('')
  const [duration, setDuration] = useState(0)

  useEffect(() => () => { void recorder.current?.cancel() }, [])

  const toggleRecording = async () => {
    if (state === 'processing') return
    if (state !== 'recording') {
      try {
        setError(''); setTranscript(''); setReport(null)
        const nextRecorder = new MicrophoneRecorder()
        recorder.current = nextRecorder
        await nextRecorder.start()
        setState('recording')
      } catch (caught) {
        setError(messageFor(caught, 'We could not access your microphone. Check browser permission and try again.'))
        setState('error')
      }
      return
    }

    try {
      setState('processing')
      const recording = await recorder.current!.stop()
      recorder.current = null
      setDuration(recording.durationSeconds)
      const result = await transcribe(recording.blob)
      setTranscript(result.text)
      if (!result.text.trim()) throw new Error('No speech was detected. Move closer to the microphone and try again.')
      setReport(await analyze(result.text))
      setState('complete')
    } catch (caught) {
      setError(messageFor(caught, 'Your recording could not be processed. Please try again.'))
      setState('error')
    }
  }

  const reset = () => { setState('ready'); setTranscript(''); setReport(null); setError(''); setDuration(0) }

  return <main className="shell">
    <header><p className="eyebrow">Pheme VA · Incident reporting</p><h1>Speak your incident report.</h1><p className="intro">Tap once to start. Tap again when you’re done. Your recording is sent securely to the local voice service for transcription.</p></header>
    <section className="voice-card" aria-live="polite">
      <p className={`state state-${state}`}><span aria-hidden="true" />{labels[state]}</p>
      <button className={`record-button ${state === 'recording' ? 'recording' : ''}`} type="button" onClick={() => void toggleRecording()} disabled={state === 'processing'} aria-label={state === 'recording' ? 'Stop recording' : 'Start recording'}>
        <span className="mic" aria-hidden="true">●</span><strong>{state === 'recording' ? 'Stop recording' : state === 'processing' ? 'Working…' : 'Start recording'}</strong>
      </button>
      <p className="help">{state === 'recording' ? 'Speak clearly, then tap Stop recording.' : 'Microphone access is requested only when you start a recording.'}</p>
    </section>
    {error && <section className="message error" role="alert"><strong>Try again</strong><p>{error}</p><button type="button" onClick={reset}>Reset</button></section>}
    {transcript && <section className="result"><div className="result-heading"><p className="eyebrow">Transcript</p>{duration > 0 && <span>{Math.ceil(duration)} sec recording</span>}</div><p className="transcript">{transcript}</p></section>}
    {report && <section className="result report"><div className="result-heading"><p className="eyebrow">Incident summary</p><button type="button" onClick={reset}>New report</button></div><dl><div><dt>Type</dt><dd>{report.incident_type}</dd></div><div><dt>Location</dt><dd>{report.location}</dd></div><div><dt>Severity</dt><dd>{report.severity}</dd></div><div className="wide"><dt>Recommended action</dt><dd>{report.recommended_action}</dd></div></dl></section>}
    <footer>This initial console uses completed recordings. Live streaming and spoken responses are not enabled yet.</footer>
  </main>
}

function messageFor(caught: unknown, fallback: string): string {
  return caught instanceof Error && caught.message ? caught.message : fallback
}
