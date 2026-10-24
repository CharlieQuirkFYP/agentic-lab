export interface SSEEvent {
  event: string
  data: string
}

// A network chunk is not an SSE frame (or even a complete UTF-8 character).
export class SSEParser {
  private buffer = ''
  private event = ''
  private data: string[] = []
  private frameSize = 0
  private readonly emit: (event: SSEEvent) => void

  constructor(emit: (event: SSEEvent) => void) {
    this.emit = emit
  }

  feed(chunk: string) {
    this.buffer += chunk
    let start = 0
    for (let i = 0; i < this.buffer.length; i++) {
      const char = this.buffer[i]
      if (char !== '\r' && char !== '\n') continue
      if (char === '\r' && i === this.buffer.length - 1) break
      this.line(this.buffer.slice(start, i))
      if (char === '\r' && this.buffer[i + 1] === '\n') i++
      start = i + 1
    }
    this.buffer = this.buffer.slice(start)
    if (this.buffer.length > 1_048_576) throw new Error('Voice event exceeds the client size limit.')
  }

  finish() {
    // A final CR is a delimiter; an unterminated event is not dispatched.
    if (this.buffer.endsWith('\r')) {
      this.line(this.buffer.slice(0, -1))
    }
    this.buffer = ''
  }

  private line(line: string) {
    if (line === '') {
      if (this.data.length) this.emit({ event: this.event || 'message', data: this.data.join('\n') })
      this.event = ''
      this.data = []
      this.frameSize = 0
      return
    }
    this.frameSize += line.length
    if (this.frameSize > 1_048_576) throw new Error('Voice event exceeds the client size limit.')
    if (line.startsWith(':')) return
    const colon = line.indexOf(':')
    const field = colon < 0 ? line : line.slice(0, colon)
    let value = colon < 0 ? '' : line.slice(colon + 1)
    if (value.startsWith(' ')) value = value.slice(1)
    if (field === 'event') this.event = value
    if (field === 'data') this.data.push(value)
  }
}

export async function consumeSSE(
  stream: ReadableStream<Uint8Array>,
  onEvent: (event: SSEEvent) => boolean | void,
) {
  const reader = stream.getReader()
  const decoder = new TextDecoder('utf-8', { fatal: true })
  let terminal = false
  const parser = new SSEParser((event) => {
    if (!terminal) terminal = onEvent(event) === true
  })
  try {
    while (!terminal) {
      const { done, value } = await reader.read()
      if (done) {
        parser.feed(decoder.decode())
        parser.finish()
        break
      }
      parser.feed(decoder.decode(value, { stream: true }))
    }
  } finally {
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
  return terminal
}
