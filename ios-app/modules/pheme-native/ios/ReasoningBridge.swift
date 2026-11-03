import Foundation
import PhemeReasoning

private struct ReasoningConfiguration: Encodable {
    let schema_version = 1
    let model_path: String
    let system_prompt: String
    let threads: Int
}

private struct ReasoningErrorEnvelope: Decodable {
    let error: BridgeFailure
}

final class ReasoningBridge: PhemeReasoningEngine, @unchecked Sendable {
    // Each caller retains Storage, so close cannot free a handle being inspected
    // or cancelled from another thread.
    private final class Storage: @unchecked Sendable {
        let handle: OpaquePointer
        init() throws {
            guard let handle = pheme_reply_create() else {
                throw BridgeFailure(code: 4, message: "Could not allocate the reasoning engine.", run_id: nil)
            }
            self.handle = handle
        }
        deinit { pheme_reply_free(handle) }
    }
    private let lock = NSLock()
    private var storage: Storage?

    init() throws { storage = try Storage() }

    private func retained() throws -> Storage {
        try lock.withLock {
            guard let storage else { throw BridgeFailure(code: 4, message: "Reasoning engine is closed.", run_id: nil) }
            return storage
        }
    }

    func inspect() throws -> ReasoningSnapshot {
        let owner = try retained()
        return try decode { pheme_reply_inspect(owner.handle, $0) }
    }

    func load(path: String, prompt: String, threads: Int) async throws -> ReasoningSnapshot {
        let owner = try retained()
        return try await execute { [self] in
            let data = try JSONEncoder().encode(ReasoningConfiguration(
                model_path: path, system_prompt: prompt, threads: threads))
            return try data.withUnsafeBytes { bytes in
                try decode { output in
                    pheme_reply_configure(owner.handle, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, output)
                }
            }
        }
    }

    func respond(text: String, requestID: String = UUID().uuidString) async throws -> ReasoningResult {
        let owner = try retained()
        return try await execute { [self] in
            let data = try JSONEncoder().encode(["request_id": requestID, "text": text])
            return try data.withUnsafeBytes { bytes in
                try decode { output in
                    pheme_reply_respond(owner.handle, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, output)
                }
            }
        }
    }

    func reset() async throws -> ReasoningSnapshot {
        let owner = try retained()
        return try await execute { [self] in try decode { pheme_reply_reset(owner.handle, $0) } }
    }

    func cancel() {
        if let owner = try? retained() { pheme_reply_cancel(owner.handle) }
    }

    func close() { lock.withLock { storage = nil } }

    private func execute<T: Sendable>(_ call: @escaping @Sendable () throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            NativeExecution.queue.async { continuation.resume(with: Result(catching: call)) }
        }
    }

    private func decode<T: Decodable>(_ call: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> Int32) throws -> T {
        var output: UnsafeMutablePointer<CChar>?
        let code = call(&output)
        defer { pheme_reply_string_free(output) }
        let data = Data((output.map { String(cString: $0) } ?? "").utf8)
        guard code == 0 else {
            if let error = try? JSONDecoder().decode(ReasoningErrorEnvelope.self, from: data) { throw error.error }
            throw BridgeFailure(code: code, message: "Native reasoning failed.", run_id: nil)
        }
        return try JSONDecoder().decode(T.self, from: data)
    }
}
