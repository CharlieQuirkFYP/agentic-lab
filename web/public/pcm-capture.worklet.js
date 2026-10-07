// Runs on the audio thread. Nothing is encoded or uploaded until recording stops.
class PCMCapture extends AudioWorkletProcessor {
  constructor() {
    super()
    this.buffer = new Float32Array(2048)
    this.offset = 0
    this.stopped = false
    this.port.onmessage = ({ data }) => {
      if (data === 'stop') {
        this.flush()
        this.stopped = true
        this.port.postMessage({ stopped: true })
      }
    }
  }

  flush() {
    if (!this.offset) return
    const samples = this.buffer.slice(0, this.offset)
    this.port.postMessage({ samples }, [samples.buffer])
    this.offset = 0
  }

  process(inputs) {
    if (this.stopped) return false
    const channels = inputs[0]
    if (!channels?.length) return true
    for (let i = 0; i < channels[0].length; i++) {
      let value = 0
      for (const channel of channels) value += channel[i]
      this.buffer[this.offset++] = value / channels.length
      if (this.offset === this.buffer.length) this.flush()
    }
    return true
  }
}

registerProcessor('pcm-capture', PCMCapture)
