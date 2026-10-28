import Foundation
import PhemeVA

/// Each queued call retains this owner until native work returns. ARC destruction
/// therefore cannot race inference, polling, or a cancelled Swift UI task.
/// Rust synchronizes engine access; the execution queue keeps blocking work off UI.
final class NativeBridge: @unchecked Sendable {
    private let handle: OpaquePointer
    private let observer: OpaquePointer
    private let executor = DispatchQueue(label: "org.agenticlab.pheme.inference", qos: .userInitiated)

    convenience init() throws {
        guard pheme_va_abi_version() == 1 else { throw BridgeFailure(code: 5, message: "Unsupported Pheme ABI version.", run_id: nil) }
        try self.init(handle: pheme_va_create())
    }

    init(handle: OpaquePointer?) throws {
        guard let handle else { throw BridgeFailure(code: 4, message: "Could not allocate the engine.", run_id: nil) }
        guard let observer = pheme_va_observer_new(handle) else {
            pheme_va_free(handle)
            throw BridgeFailure(code: 4, message: "Could not allocate the metrics observer.", run_id: nil)
        }
        self.handle = handle
        self.observer = observer
    }
    deinit {
        // All methods retain self; no outstanding caller can reach this point.
        pheme_va_observer_free(observer)
        pheme_va_free(handle)
    }

    func configure(_ configuration: BridgeConfiguration) async throws -> BridgeSnapshot {
        try await execute { [self] in
            let data = try JSONEncoder().encode(configuration)
            return try data.withUnsafeBytes { bytes in
                try decode { pheme_va_configure(handle, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, $0) }
            }
        }
    }

    func transcribe(_ audio: CapturedAudio) async throws -> Transcription {
        try await execute { [self] in
            try audio.samples.withUnsafeBufferPointer { samples in
                try decode { pheme_va_transcribe_v1(handle, samples.baseAddress, samples.count, audio.sampleRate, audio.channels, $0) }
            }
        }
    }

    func inspect() throws -> BridgeSnapshot {
        try decode { pheme_va_inspect(handle, $0) }
    }

    // Admission is already stopped by the host. close does not free the handle;
    // ARC releases it only after all retained native calls have returned.
    func close() throws {
        _ = try raw { pheme_va_close(handle, $0) }
    }

    /// Called by exactly one service poller, independently of the inference queue.
    func poll() throws -> BridgePoll {
        let snapshot = try inspect()
        let status: ObserverSnapshot = try decode { pheme_va_observer_inspect(observer, $0) }
        let batches = try raw { pheme_va_observer_drain(observer, $0) }
        return BridgePoll(snapshot: snapshot, observer: status, batchesJSON: batches)
    }

    private func execute<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            executor.async { continuation.resume(with: Result(catching: work)) }
        }
    }

    private func decode<T: Decodable>(_ call: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> Int32) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(raw(call).utf8))
    }

    private func raw(_ call: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> Int32) throws -> String {
        var output: UnsafeMutablePointer<CChar>?
        let code = call(&output)
        defer { pheme_va_string_free(output) }
        let value = output.map { String(cString: $0) } ?? ""
        guard code == 0 else {
            struct ErrorEnvelope: Decodable { let error: BridgeFailure }
            if let error = try? JSONDecoder().decode(ErrorEnvelope.self, from: Data(value.utf8)) { throw error.error }
            throw BridgeFailure(code: code, message: "Native operation failed (code \(code)).", run_id: nil)
        }
        guard output != nil else { throw BridgeFailure(code: 2, message: "Missing native output.", run_id: nil) }
        return value
    }
}
