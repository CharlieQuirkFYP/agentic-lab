import { useEffect, useRef, useState } from 'react'

import { MicrophoneRecorder } from './audio'
import { analyze, transcribe, type IncidentReport } from './phemeApi'

type VoiceState = 'ready' | 'recording' | 'processing' | 'error'
type ChatMessage = {
  id: string
  role: 'user' | 'assistant'
  kind: 'voice' | 'text'
  createdAt: string
  audioUrl?: string
  durationSeconds?: number
  text?: string
  report?: IncidentReport
}

const STORAGE_KEY = 'pheme-voice-chat'

export default function App() {
  const recorder = useRef<MicrophoneRecorder | null>(null)
  const [state, setState] = useState<VoiceState>('ready')
  const [messages, setMessages] = useState<ChatMessage[]>(() => loadMessages())
  const [error, setError] = useState('')

  useEffect(() => { localStorage.setItem(STORAGE_KEY, JSON.stringify(messages)) }, [messages])
  useEffect(() => () => { void recorder.current?.cancel() }, [])

  const toggleRecording = async () => {
    if (state === 'processing') return
    if (state !== 'recording') {
      try {
        setError('')
        const nextRecorder = new MicrophoneRecorder()
        recorder.current = nextRecorder
        await nextRecorder.start()
        setState('recording')
      } catch (caught) {
        recorder.current = null
        setError(messageFor(caught, 'We could not access your microphone. Check browser permission and try again.'))
        setState('error')
      }
      return
    }

    try {
      setState('processing')
      const recording = await recorder.current!.stop()
      recorder.current = null
      const audioUrl = await blobToDataUrl(recording.blob)
      setMessages((current) => [...current, { id: crypto.randomUUID(), role: 'user', kind: 'voice', createdAt: new Date().toISOString(), audioUrl, durationSeconds: recording.durationSeconds }])

      const result = await transcribe(recording.blob)
      if (!result.text.trim()) throw new Error('No speech was detected. Move closer to the microphone and try again.')
      const report = await analyze(result.text)
      setMessages((current) => [...current, { id: crypto.randomUUID(), role: 'assistant', kind: 'text', createdAt: new Date().toISOString(), text: result.text, report }])
      setState('ready')
    } catch (caught) {
      setError(messageFor(caught, 'Your recording could not be processed. Please try again.'))
      setState('error')
    }
  }

  const clearChat = () => { setMessages([]); setError(''); setState('ready') }

  return <main className="shell">
    <header className="chat-header">
      <div><p className="eyebrow">Pheme VA · Incident reporting</p><h1>Voice reports</h1><p className="intro">Send a voice message and Pheme will transcribe it and return an incident summary.</p></div>
      {messages.length > 0 && <button className="clear-button" type="button" onClick={clearChat}>Clear chat</button>}
    </header>
    <section className="chat" aria-live="polite">
      {messages.length === 0 && <div className="empty-chat"><div className="empty-icon" aria-hidden="true">⌁</div><strong>Start a conversation</strong><p>Tap the microphone below to send your first report.</p></div>}
      {messages.map((message) => <ChatBubble key={message.id} message={message} />)}
    </section>
    {error && <section className="message error" role="alert"><strong>Try again</strong><p>{error}</p><button type="button" onClick={() => { setError(''); setState('ready') }}>Dismiss</button></section>}
    <section className="composer" aria-label="Voice message composer">
      <p className={`state state-${state}`}><span aria-hidden="true" />{state === 'recording' ? 'Listening…' : state === 'processing' ? 'Processing your report…' : state === 'error' ? 'Microphone unavailable' : 'Ready to listen'}</p>
      <button className={`record-button ${state === 'recording' ? 'recording' : ''}`} type="button" onClick={() => void toggleRecording()} disabled={state === 'processing'} aria-label={state === 'recording' ? 'Stop recording' : 'Start recording'}>
        <span className="mic" aria-hidden="true">●</span><strong>{state === 'recording' ? 'Stop and send' : state === 'processing' ? 'Working…' : 'Record a message'}</strong>
      </button>
      <p className="help">{state === 'recording' ? 'Tap to stop and send your voice message.' : 'Microphone access is requested when you record.'}</p>
    </section>
    <footer>Your chat is kept in this browser. Voice messages stay available to play after a refresh.</footer>
  </main>
}

function ChatBubble({ message }: { message: ChatMessage }) {
  return <article className={`bubble-row ${message.role}`}><div className={`bubble ${message.role}`}>
    {message.kind === 'voice' && message.audioUrl ? <div className="voice-message"><span className="voice-label">Voice message</span><audio controls preload="metadata" src={message.audioUrl} /><span className="duration">{Math.ceil(message.durationSeconds ?? 0)} sec</span></div> : <>
      <p className="bubble-label">Transcript</p><p className="bubble-text">{message.text}</p>
      {message.report && <div className="report"><p className="bubble-label">Incident summary</p><dl><div><dt>Type</dt><dd>{message.report.incident_type}</dd></div><div><dt>Location</dt><dd>{message.report.location}</dd></div><div><dt>Severity</dt><dd>{message.report.severity}</dd></div><div className="wide"><dt>Recommended action</dt><dd>{message.report.recommended_action}</dd></div></dl></div>}
    </>}
    <time dateTime={message.createdAt}>{formatTime(message.createdAt)}</time>
  </div></article>
}

function loadMessages(): ChatMessage[] { try { return JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]') as ChatMessage[] } catch { return [] } }
function blobToDataUrl(blob: Blob): Promise<string> { return new Promise((resolve, reject) => { const reader = new FileReader(); reader.onload = () => resolve(String(reader.result)); reader.onerror = () => reject(new Error('The recording could not be saved.')); reader.readAsDataURL(blob) }) }
function formatTime(value: string): string { return new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' }).format(new Date(value)) }
function messageFor(caught: unknown, fallback: string): string {
  if (caught instanceof DOMException && caught.name === 'NotAllowedError') return 'Microphone access was declined. Press Record a message to request it again. If the browser has marked this site as blocked, allow the microphone from the lock icon in the address bar, then retry.'
  return caught instanceof Error && caught.message ? caught.message : fallback
}
