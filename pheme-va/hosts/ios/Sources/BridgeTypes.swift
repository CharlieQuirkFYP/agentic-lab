import Foundation

struct BridgeConfiguration: Codable, Equatable, Sendable {
    var schema_version: UInt32 = 1
    var model_path: String
    var language: String? = nil
    var dictionary: [String] = []
    var threads: Int = 4
    var max_audio_seconds: Int = 120
    var metrics_enabled: Bool = true
}
struct BridgeFailure: Error, Codable, LocalizedError, Sendable {
    let code: Int32
    let message: String
    let run_id: String?
    var errorDescription: String? { message }
}
struct Capabilities: Codable, Sendable {
    let whisper: Bool
    let whisper_metal: Bool
    let whisper_coreml: Bool
    let metrics_output: Bool
}
struct BridgeSnapshot: Codable, Sendable {
    let schema_version: UInt32
    let state: String
    let ready: Bool
    let capabilities: Capabilities
    let configuration: BridgeConfiguration?
    let active_run: String?
    let last_failure: BridgeFailure?
}
struct ObserverSnapshot: Codable, Sendable {
    let capacity_events: Int
    let pending_events: Int
    let dropped_events: UInt64
}
struct Transcription: Codable, Sendable {
    let schema_version: UInt32
    let run_id: String
    let result: ResultBody
    struct ResultBody: Codable, Sendable {
        let text: String
        let status: String
        let model_id: String
        let processing_time_ms: Double
    }
}
struct CapturedAudio: Sendable {
    let samples: [Float]
    let sampleRate: UInt32
    let channels: UInt16
}
struct BridgePoll: Sendable {
    let snapshot: BridgeSnapshot
    let observer: ObserverSnapshot
    // Keep this exact payload for fan-out/retry. Do not re-drain for each consumer.
    let batchesJSON: String
}
