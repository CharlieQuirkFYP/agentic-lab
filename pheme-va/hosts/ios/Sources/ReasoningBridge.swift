#if PHEME_REASONING
import Foundation
import PhemeReasoning

struct ReasoningSnapshot: Decodable, Sendable {
    let ready: Bool
    let state: String
    let model_path: String?
    let threads: Int?
    let active_run: String?
    let history_turns: Int
    let load_time_ms: UInt64
}

struct ReasoningResult: Decodable, Sendable {
    let run_id: String
    let text: String
    let processing_time_ms: UInt64
}

private struct ReasoningConfiguration: Encodable {
    let schema_version = 1
    let model_path: String
    let system_prompt: String
    let threads: Int
}

private struct ReasoningErrorEnvelope: Decodable {
    let error: BridgeFailure
}

final class ReasoningBridge: @unchecked Sendable {
    private let handle: OpaquePointer

    init() throws {
        guard let handle = pheme_reply_create() else {
            throw BridgeFailure(code: 4, message: "Could not allocate the reasoning engine.", run_id: nil)
        }
        self.handle = handle
    }
    deinit { pheme_reply_free(handle) }

    func inspect() throws -> ReasoningSnapshot {
        try decode { pheme_reply_inspect(handle, $0) }
    }

    func load(path: String, prompt: String, threads: Int) async throws -> ReasoningSnapshot {
        try await execute { [self] in
            let data = try JSONEncoder().encode(ReasoningConfiguration(
                model_path: path, system_prompt: prompt, threads: threads))
            return try data.withUnsafeBytes { bytes in
                try decode { output in
                    pheme_reply_configure(handle, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, output)
                }
            }
        }
    }

    func respond(text: String, requestID: String = UUID().uuidString) async throws -> ReasoningResult {
        try await execute { [self] in
            let data = try JSONEncoder().encode(["request_id": requestID, "text": text])
            return try data.withUnsafeBytes { bytes in
                try decode { output in
                    pheme_reply_respond(handle, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, output)
                }
            }
        }
    }

    func reset() async throws -> ReasoningSnapshot {
        try await execute { [self] in try decode { pheme_reply_reset(handle, $0) } }
    }

    func cancel() { pheme_reply_cancel(handle) }

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
#endif
