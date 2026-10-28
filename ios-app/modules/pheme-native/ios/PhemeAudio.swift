import AVFoundation

enum PhemeAudio {
    static func decode(_ url: URL, maxSeconds: Int) throws -> CapturedAudio {
        do { return try decodeWAV(url, maxSeconds: maxSeconds) }
        catch let failure as PhemeLocalFailure { throw failure }
        catch { throw PhemeLocalFailure(code: "invalid_audio", message: "Could not decode this WAV: " + error.localizedDescription) }
    }

    private static func decodeWAV(_ url: URL, maxSeconds: Int) throws -> CapturedAudio {
        let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        guard size > 0, size <= 32 * 1024 * 1024, url.pathExtension.lowercased() == "wav" else {
            throw PhemeLocalFailure(code: "invalid_audio", message: "Choose a non-empty WAV file no larger than 32 MiB.")
        }
        let input = try FileHandle(forReadingFrom: url)
        defer { try? input.close() }
        let header = try input.read(upToCount: 12) ?? Data()
        guard header.count == 12, String(data: header.prefix(4), encoding: .ascii) == "RIFF",
              String(data: header.suffix(4), encoding: .ascii) == "WAVE" else {
            throw PhemeLocalFailure(code: "invalid_audio", message: "The selected file is not a RIFF/WAV recording.")
        }
        let file = try AVAudioFile(forReading: url, commonFormat: .pcmFormatFloat32, interleaved: false)
        let format = file.processingFormat
        let channels = Int(format.channelCount)
        let frames = file.length
        guard format.sampleRate.isFinite, (8000...192000).contains(format.sampleRate),
              (1...8).contains(channels), frames > 0, frames <= 16_777_216 / Int64(channels),
              Double(frames) / format.sampleRate <= Double(maxSeconds),
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(frames)) else {
            throw PhemeLocalFailure(code: "audio_limit", message: "Audio exceeds the configured duration or supported sample/channel limits.")
        }
        try file.read(into: buffer)
        guard let planes = buffer.floatChannelData, buffer.frameLength == AVAudioFrameCount(frames) else {
            throw PhemeLocalFailure(code: "invalid_audio", message: "Could not decode the complete WAV recording.")
        }
        var samples = [Float](repeating: 0, count: Int(frames) * channels)
        for frame in 0..<Int(frames) {
            for channel in 0..<channels {
                let value = planes[channel][frame]
                guard value.isFinite else { throw PhemeLocalFailure(code: "invalid_audio", message: "Audio contains non-finite samples.") }
                samples[frame * channels + channel] = value
            }
        }
        return CapturedAudio(samples: samples, sampleRate: UInt32(format.sampleRate), channels: UInt16(channels))
    }
}
