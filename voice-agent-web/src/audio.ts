export type Recording = {
  blob: Blob
  durationSeconds: number
}

export class MicrophoneRecorder {
  private audioContext: AudioContext | undefined
  private chunks: Float32Array[] = []
  private processor: ScriptProcessorNode | undefined
  private source: MediaStreamAudioSourceNode | undefined
  private stream: MediaStream | undefined
  private startedAt = 0

  async start(): Promise<void> {
    if (!navigator.mediaDevices?.getUserMedia) {
      throw new Error('This browser does not support microphone recording.')
    }

    this.stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    })
    this.audioContext = new AudioContext()
    await this.audioContext.resume()
    this.source = this.audioContext.createMediaStreamSource(this.stream)
    this.processor = this.audioContext.createScriptProcessor(4096, 1, 1)
    const silentOutput = this.audioContext.createGain()
    silentOutput.gain.value = 0
    this.chunks = []
    this.processor.onaudioprocess = (event) => {
      this.chunks.push(new Float32Array(event.inputBuffer.getChannelData(0)))
    }
    this.source.connect(this.processor)
    this.processor.connect(silentOutput)
    silentOutput.connect(this.audioContext.destination)
    this.startedAt = Date.now()
  }

  async stop(): Promise<Recording> {
    if (!this.audioContext || !this.stream || !this.processor || !this.source) {
      throw new Error('No active recording was found.')
    }

    const durationSeconds = (Date.now() - this.startedAt) / 1000
    const sampleRate = this.audioContext.sampleRate
    this.processor.disconnect()
    this.source.disconnect()
    this.stream.getTracks().forEach((track) => track.stop())
    await this.audioContext.close()
    const samples = joinChunks(this.chunks)
    this.audioContext = undefined
    this.processor = undefined
    this.source = undefined
    this.stream = undefined
    return { blob: encodeWav(samples, sampleRate), durationSeconds }
  }

  async cancel(): Promise<void> {
    this.processor?.disconnect()
    this.source?.disconnect()
    this.stream?.getTracks().forEach((track) => track.stop())
    await this.audioContext?.close()
    this.audioContext = undefined
    this.processor = undefined
    this.source = undefined
    this.stream = undefined
    this.chunks = []
  }
}

function joinChunks(chunks: Float32Array[]): Float32Array {
  const length = chunks.reduce((sum, chunk) => sum + chunk.length, 0)
  const samples = new Float32Array(length)
  let offset = 0
  chunks.forEach((chunk) => { samples.set(chunk, offset); offset += chunk.length })
  return samples
}

function encodeWav(samples: Float32Array, sampleRate: number): Blob {
  const buffer = new ArrayBuffer(44 + samples.length * 2)
  const view = new DataView(buffer)
  writeText(view, 0, 'RIFF')
  view.setUint32(4, 36 + samples.length * 2, true)
  writeText(view, 8, 'WAVE')
  writeText(view, 12, 'fmt ')
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true)
  view.setUint16(22, 1, true)
  view.setUint32(24, sampleRate, true)
  view.setUint32(28, sampleRate * 2, true)
  view.setUint16(32, 2, true)
  view.setUint16(34, 16, true)
  writeText(view, 36, 'data')
  view.setUint32(40, samples.length * 2, true)
  samples.forEach((sample, index) => view.setInt16(44 + index * 2, Math.max(-1, Math.min(1, sample)) * 0x7fff, true))
  return new Blob([buffer], { type: 'audio/wav' })
}

function writeText(view: DataView, offset: number, text: string): void {
  Array.from(text).forEach((character, index) => view.setUint8(offset + index, character.charCodeAt(0)))
}
