import AVFoundation
import Foundation

/// The tap only copies bounded PCM. Inference starts after recording stops.
final class AudioRecorder: @unchecked Sendable {
    private let engine = AVAudioEngine()
    private let lock = NSLock()
    private var samples: [Float] = []
    private var format: AVAudioFormat?
    private var limit = 0
    private var accepting = false
    private var installedTap = false

    @MainActor
    func start(maxSeconds: Int, limitReached: @escaping @MainActor @Sendable () -> Void) throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.record, mode: .measurement)
        try session.setActive(true)
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.commonFormat == .pcmFormatFloat32, format.sampleRate >= 8000,
              format.sampleRate <= 192000, (1...8).contains(format.channelCount) else {
            try? session.setActive(false)
            throw BridgeFailure(code: 1, message: "This microphone format is unsupported.", run_id: nil)
        }
        self.format = format
        lock.lock()
        samples = []
        limit = min(16 * 1024 * 1024, Int(format.sampleRate) * Int(format.channelCount) * maxSeconds)
        samples.reserveCapacity(limit)
        accepting = true
        lock.unlock()
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, _ in
            guard let self, let data = buffer.floatChannelData else { return }
            self.lock.lock()
            guard self.accepting else { self.lock.unlock(); return }
            let channels = Int(buffer.format.channelCount)
            let frames = min(Int(buffer.frameLength), (self.limit - self.samples.count) / channels)
            for frame in 0..<frames {
                for channel in 0..<channels {
                    self.samples.append(buffer.format.isInterleaved ? data[0][frame * channels + channel] : data[channel][frame])
                }
            }
            let full = self.samples.count + channels > self.limit
            if full { self.accepting = false }
            self.lock.unlock()
            if full { Task { @MainActor in limitReached() } }
        }
        installedTap = true
        do {
            engine.prepare()
            try engine.start()
        } catch {
            _ = stop()
            throw error
        }
    }

    @MainActor
    func stop() -> CapturedAudio? {
        lock.lock()
        accepting = false
        lock.unlock()
        if installedTap {
            engine.inputNode.removeTap(onBus: 0)
            installedTap = false
        }
        engine.stop()
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        lock.lock()
        defer { lock.unlock() }
        guard let format, !samples.isEmpty else { samples = []; return nil }
        let audio = CapturedAudio(samples: samples, sampleRate: UInt32(format.sampleRate), channels: UInt16(format.channelCount))
        samples = []
        return audio
    }
}
