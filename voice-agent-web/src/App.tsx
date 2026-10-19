import { useEffect, useRef, useState } from 'react'

import { MicrophoneRecorder } from './audio'
import { validateWavFile } from './fileValidation'
import { analyze, transcribe, type IncidentReport } from './phemeApi'

type VoiceState = 'ready' | 'recording' | 'processing' | 'error'
type ErrorKind = 'microphone' | 'file' | 'no-speech' | null
type ChatMessage = {
  id: string
  role: 'user' | 'assistant'
  kind: 'voice' | 'text'
  createdAt: string
  audioUrl?: string
  fileName?: string
  durationSeconds?: number
  text?: string
  report?: IncidentReport
}

const STORAGE_KEY = 'pheme-voice-chat'

export default function App() {
  const recorder = useRef<MicrophoneRecorder | null>(null)
  const processingAbortController = useRef<AbortController | null>(null)
  const [state, setState] = useState<VoiceState>('ready')
  const [messages, setMessages] = useState<ChatMessage[]>(() => loadMessages())
  const [error, setError] = useState('')
  const [errorKind, setErrorKind] = useState<ErrorKind>(null)
  const [isDraggingFile, setIsDraggingFile] = useState(false)
  const [microphoneBlocked, setMicrophoneBlocked] = useState(false)
  const dragDepth = useRef(0)

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(messages))
    } catch (caught) {
      // Large WAV data URLs can exceed the browser's localStorage quota. Keep
      // the current chat usable even when persistence is unavailable.
      console.warn('Could not persist chat history:', caught)
    }
  }, [messages])
  useEffect(() => () => { void recorder.current?.cancel() }, [])
  useEffect(() => {
    let permission: PermissionStatus | undefined
    const updatePermission = () => setMicrophoneBlocked(permission?.state === 'denied')
    if (!navigator.permissions?.query) return
    void navigator.permissions.query({ name: 'microphone' as PermissionName }).then((result) => {
      permission = result
      updatePermission()
      permission.addEventListener('change', updatePermission)
    }).catch(() => undefined)
    return () => permission?.removeEventListener('change', updatePermission)
  }, [])

  const sendAudio = async (blob: Blob, durationSeconds: number, fileName?: string) => {
    const controller = new AbortController()
    processingAbortController.current = controller
    const audioUrl = await blobToDataUrl(blob)
    setMessages((current) => [...current, { id: crypto.randomUUID(), role: 'user', kind: 'voice', createdAt: new Date().toISOString(), audioUrl, fileName, durationSeconds }])

    const result = await transcribe(blob, controller.signal)
    if (!result.text.trim()) throw new Error('No speech was detected. Move closer to the microphone and try again.')
    const report = await analyze(result.text, controller.signal)
    setMessages((current) => [...current, { id: crypto.randomUUID(), role: 'assistant', kind: 'text', createdAt: new Date().toISOString(), text: result.text, report }])
    processingAbortController.current = null
  }

  const toggleRecording = async () => {
    if (state === 'processing') return
    if (state !== 'recording') {
      try {
        setError('')
        setErrorKind(null)
        const nextRecorder = new MicrophoneRecorder()
        recorder.current = nextRecorder
        await nextRecorder.start()
        setMicrophoneBlocked(false)
        setState('recording')
      } catch (caught) {
        recorder.current = null
        setError(messageFor(caught, 'We could not access your microphone. Check browser permission and try again.'))
        setErrorKind('microphone')
        setMicrophoneBlocked(true)
        setState('error')
      }
      return
    }

    try {
      setState('processing')
      const recording = await recorder.current!.stop()
      recorder.current = null
      await sendAudio(recording.blob, recording.durationSeconds)
      setState('ready')
    } catch (caught) {
      if (isAbortError(caught)) { setState('ready'); return }
      setError(messageFor(caught, 'Your recording could not be processed. Please try again.'))
      setErrorKind(caught instanceof Error && caught.message.startsWith('No speech') ? 'no-speech' : 'microphone')
      setState('error')
    }
  }

  const processFile = async (file: File) => {
    const validationError = validateWavFile(file)
    if (validationError) {
      setError(validationError)
      setErrorKind('file')
      setState('error')
      return
    }

    try {
      setError('')
      setErrorKind(null)
      setState('processing')
      const durationSeconds = await getAudioDuration(file)
      if (durationSeconds > 120) throw new Error('This WAV file is longer than the 120-second limit.')
      await sendAudio(file, durationSeconds, file.name)
      setState('ready')
    } catch (caught) {
      if (isAbortError(caught)) { setState('ready'); return }
      setError(messageFor(caught, 'Your WAV file could not be processed. Please try another file.'))
      setErrorKind(caught instanceof Error && caught.message.startsWith('No speech') ? 'no-speech' : 'file')
      setState('error')
    }
  }

  const handleFileSelected = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (file) await processFile(file)
  }

  const handleFileDrop = async (event: React.DragEvent<HTMLElement>) => {
    event.preventDefault()
    dragDepth.current = 0
    setIsDraggingFile(false)
    if (state === 'recording' || state === 'processing') return
    const file = event.dataTransfer.files[0]
    if (file) await processFile(file)
  }

  const clearChat = () => { setMessages([]); setError(''); setErrorKind(null); setState('ready') }

  const cancelProcessing = () => {
    processingAbortController.current?.abort()
    processingAbortController.current = null
    setState('ready')
  }

  const retryMicrophone = async () => {
    setError('')
    setErrorKind(null)
    setState('ready')
    await toggleRecording()
  }

  const handleDragEnter = (event: React.DragEvent<HTMLElement>) => {
    if (!event.dataTransfer.types.includes('Files')) return
    event.preventDefault()
    dragDepth.current += 1
    setIsDraggingFile(true)
  }

  const handleDragOver = (event: React.DragEvent<HTMLElement>) => {
    if (!event.dataTransfer.types.includes('Files')) return
    event.preventDefault()
    event.dataTransfer.dropEffect = state === 'recording' || state === 'processing' ? 'none' : 'copy'
    setIsDraggingFile(true)
  }

  const handleDragLeave = (event: React.DragEvent<HTMLElement>) => {
    if (!event.dataTransfer.types.includes('Files')) return
    event.preventDefault()
    dragDepth.current = Math.max(0, dragDepth.current - 1)
    if (dragDepth.current === 0) setIsDraggingFile(false)
  }

  return <main className="shell" onDragEnter={handleDragEnter} onDragOver={handleDragOver} onDragLeave={handleDragLeave} onDrop={(event) => void handleFileDrop(event)}>
    {isDraggingFile && <div className="drop-overlay" aria-live="polite"><div className="drop-overlay-card"><span className="drop-overlay-icon" aria-hidden="true">↓</span><strong>Drop to upload a WAV file</strong><span>Maximum size: 2 MB</span></div></div>}
    <header className="chat-header">
      <div><p className="eyebrow">Pheme VA · Incident reporting</p><h1>Voice reports</h1><p className="intro">Send a voice message and Pheme will transcribe it and return an incident summary.</p></div>
      {messages.length > 0 && <button className="clear-button" type="button" onClick={clearChat}>Clear chat</button>}
    </header>
    <section className="chat" aria-live="polite">
      {messages.length === 0 && <div className="empty-chat"><div className="empty-icon" aria-hidden="true">⌁</div><strong>Start a conversation</strong><p>Tap the microphone below to send your first report.</p></div>}
      {messages.map((message) => <ChatBubble key={message.id} message={message} />)}
    </section>
    {error && <div className="error-modal-backdrop" role="presentation"><section className="error-modal" role="alertdialog" aria-modal="true" aria-labelledby="error-modal-title"><div className="error-modal-icon" aria-hidden="true">!</div><strong id="error-modal-title">{errorKind === 'file' ? 'Upload failed' : errorKind === 'no-speech' ? 'No speech detected' : 'Microphone unavailable'}</strong><p>{errorKind === 'microphone' ? `${error} If your browser remembers the denial, allow microphone access from the lock icon or site settings, then try again.` : error}</p><div className="error-modal-actions">{errorKind === 'microphone' && <button type="button" onClick={() => void retryMicrophone()}>Try microphone again</button>}<button className="secondary" type="button" onClick={() => { setError(''); setErrorKind(null); setState('ready') }}>Okay</button></div></section></div>}
    <section className="composer" aria-label="Voice message composer">
      <p className={`state state-${state}`}><span aria-hidden="true" />{state === 'recording' ? 'Listening…' : state === 'processing' ? 'Processing your report…' : state === 'error' ? (errorKind === 'file' ? 'Upload unavailable' : 'Microphone unavailable') : microphoneBlocked ? 'Microphone blocked — upload a WAV file or click to retry' : 'Ready to listen'}</p>
      <button className={`record-button ${state === 'recording' ? 'recording' : ''} ${state === 'processing' ? 'processing' : ''} ${microphoneBlocked ? 'blocked' : ''}`} type="button" onClick={() => state === 'processing' ? cancelProcessing() : void toggleRecording()} aria-label={state === 'processing' ? 'Cancel processing' : microphoneBlocked ? 'Retry microphone access' : state === 'recording' ? 'Stop recording' : 'Start recording'}>
        <span className="mic" aria-hidden="true">●</span><strong>{state === 'recording' ? 'Stop and send' : state === 'processing' ? 'Cancel' : microphoneBlocked ? 'Microphone blocked · click to retry' : 'Record a message'}</strong>
      </button>
      <label className={`upload-button ${isDraggingFile ? 'dragging' : ''}`}>
        <span>{isDraggingFile ? 'Drop WAV file here' : 'Upload or drop a WAV file'}</span>
        <input type="file" accept=".wav,audio/wav,audio/x-wav" onChange={(event) => void handleFileSelected(event)} disabled={state === 'recording' || state === 'processing'} />
      </label>
      <p className="help">{state === 'recording' ? 'Tap to stop and send your voice message.' : 'Record a message or upload a WAV file (up to 2 MB and 120 seconds).'}</p>
    </section>
    <footer>Your chat is kept in this browser. Voice messages stay available to play after a refresh.</footer>
  </main>
}

function ChatBubble({ message }: { message: ChatMessage }) {
  return <article className={`bubble-row ${message.role}`}><div className={`bubble ${message.role}`}>
    {message.kind === 'voice' && message.audioUrl ? <div className="voice-message"><span className="voice-label">{message.fileName ?? 'Voice message'}</span><audio controls preload="metadata" src={message.audioUrl} /><div className="voice-actions"><span className="duration">{Math.ceil(message.durationSeconds ?? 0)} sec</span><a className="download-link" href={message.audioUrl} download={message.fileName ?? 'voice-message.wav'}>Download WAV</a></div></div> : <>
      <p className="bubble-label">Transcript</p><p className="bubble-text">{message.text}</p>
      {message.report && <div className="report"><p className="bubble-label">Incident summary</p><dl><div><dt>Type</dt><dd>{message.report.incident_type}</dd></div><div><dt>Location</dt><dd>{message.report.location}</dd></div><div><dt>Severity</dt><dd>{message.report.severity}</dd></div><div className="wide"><dt>Recommended action</dt><dd>{message.report.recommended_action}</dd></div></dl></div>}
    </>}
    <time dateTime={message.createdAt}>{formatTime(message.createdAt)}</time>
  </div></article>
}

function loadMessages(): ChatMessage[] { try { return JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]') as ChatMessage[] } catch { return [] } }
function blobToDataUrl(blob: Blob): Promise<string> { return new Promise((resolve, reject) => { const reader = new FileReader(); reader.onload = () => resolve(String(reader.result)); reader.onerror = () => reject(new Error('The recording could not be saved.')); reader.readAsDataURL(blob) }) }
function getAudioDuration(file: File): Promise<number> { return new Promise((resolve, reject) => { const url = URL.createObjectURL(file); const audio = new Audio(); audio.onloadedmetadata = () => { URL.revokeObjectURL(url); if (!Number.isFinite(audio.duration)) reject(new Error('The WAV duration could not be determined.')); else resolve(audio.duration) }; audio.onerror = () => { URL.revokeObjectURL(url); reject(new Error('The selected file is not a readable WAV file.')) }; audio.src = url }) }
function formatTime(value: string): string { return new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' }).format(new Date(value)) }
function messageFor(caught: unknown, fallback: string): string {
  if (caught instanceof DOMException && caught.name === 'NotAllowedError') return 'Microphone access was declined. Press Record a message to request it again. If the browser has marked this site as blocked, allow the microphone from the lock icon in the address bar, then retry.'
  return caught instanceof Error && caught.message ? caught.message : fallback
}
function isAbortError(caught: unknown): boolean { return caught instanceof DOMException && caught.name === 'AbortError' }
