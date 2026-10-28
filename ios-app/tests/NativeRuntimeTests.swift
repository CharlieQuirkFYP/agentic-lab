import Foundation
import CryptoKit

struct TestFailure: Error { let message: String }
func expect(_ condition: @autoclosure () -> Bool, _ message: String) throws {
    if !condition() { throw TestFailure(message: message) }
}

func waitUntil(_ condition: @Sendable () -> Bool) async throws {
    let deadline = Date().addingTimeInterval(5)
    while !condition() && Date() < deadline { try await Task.sleep(nanoseconds: 1_000_000) }
    try expect(condition(), "Timed out waiting for the fake engine lifecycle transition")
}

final class FakeEngine: PhemeEngine, @unchecked Sendable {
    private let lock = NSLock()
    private var configuration: BridgeConfiguration?
    private var waiter: CheckedContinuation<Void, Never>?
    private var shouldPause = false
    private var closes = 0
    private var loads = 0
    private var runs = 0
    private var holdClose = false
    private let closeGate = DispatchSemaphore(value: 0)
    var closeCount: Int { lock.withLock { closes } }
    var loadCount: Int { lock.withLock { loads } }
    var paused: Bool { lock.withLock { waiter != nil } }
    func pauseNext() { lock.withLock { shouldPause = true } }
    func pauseClose() { lock.withLock { holdClose = true } }
    func releaseClose() { closeGate.signal() }
    func release() { let continuation = lock.withLock { let value = waiter; waiter = nil; return value }; continuation?.resume() }
    func configure(_ configuration: BridgeConfiguration) async throws -> BridgeSnapshot {
        lock.withLock { self.configuration = configuration; loads += 1 }
        return try inspect()
    }
    func transcribe(_ audio: CapturedAudio) async throws -> Transcription {
        let pause = lock.withLock { runs += 1; return shouldPause }
        if pause { await withCheckedContinuation { continuation in lock.withLock { waiter = continuation; shouldPause = false } } }
        return Transcription(schema_version: 1, run_id: "local-test-\(lock.withLock { runs })",
            result: .init(text: "Incident near the entrance", status: "accepted", model_id: "fake", processing_time_ms: 1))
    }
    func inspect() throws -> BridgeSnapshot {
        let selected = lock.withLock { configuration }
        return BridgeSnapshot(schema_version: 1, state: selected == nil ? "unavailable" : "ready", ready: selected != nil,
            capabilities: .init(whisper: true, whisper_metal: false, whisper_coreml: false, metrics_output: true), configuration: selected, active_run: nil, last_failure: nil)
    }
    func poll() throws -> BridgePoll {
        BridgePoll(snapshot: try inspect(), observer: .init(capacity_events: 512, pending_events: 0, dropped_events: 2), batchesJSON: "[ {\"run_id\": \"local-test-1\"} ]")
    }
    func close() throws {
        let pause = lock.withLock { closes += 1; configuration = nil; return holdClose }
        if pause { closeGate.wait() }
    }
}

func failure(_ code: String, _ work: @Sendable () async throws -> Void) async throws {
    do { try await work(); throw TestFailure(message: "Expected \(code)") }
    catch let error as PhemeLocalFailure { try expect(error.code == code, "Expected \(code), got \(error.code)") }
}

@main struct RuntimeTests {
    static func main() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let modelFile = root.appendingPathComponent("fixture.bin")
        let wav = root.appendingPathComponent("test.wav")
        try Data("abc".utf8).write(to: modelFile); try Data([0]).write(to: wav)
        // Exercise the real host decoder independently of fake engine inference.
        var pcm = Data()
        func appendLE<T: FixedWidthInteger>(_ value: T) { var little = value.littleEndian; withUnsafeBytes(of: &little) { pcm.append(contentsOf: $0) } }
        pcm.append(Data("RIFF".utf8)); appendLE(UInt32(36 + 64000)); pcm.append(Data("WAVEfmt ".utf8))
        appendLE(UInt32(16)); appendLE(UInt16(1)); appendLE(UInt16(2)); appendLE(UInt32(16000)); appendLE(UInt32(64000)); appendLE(UInt16(4)); appendLE(UInt16(16)); pcm.append(Data("data".utf8)); appendLE(UInt32(64000))
        for _ in 0..<16000 { appendLE(Int16(1000)); appendLE(Int16(-2000)) }
        try pcm.write(to: wav)
        let decoded = try PhemeAudio.decode(wav, maxSeconds: 1)
        try expect(decoded.channels == 2 && decoded.sampleRate == 16000 && decoded.samples.count == 32000, "WAV shape changed")
        try expect(abs(decoded.samples[0] - 1000.0/32768.0) < 0.00001 && abs(decoded.samples[1] + 2000.0/32768.0) < 0.00001, "Stereo samples were not interleaved correctly")
        try await failure("audio_limit") { _ = try PhemeAudio.decode(wav, maxSeconds: 0) }
        let badWAV = root.appendingPathComponent("bad.wav"); try Data([0]).write(to: badWAV)
        try await failure("invalid_audio") { _ = try PhemeAudio.decode(badWAV, maxSeconds: 1) }
        let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        let engine = FakeEngine()
        let directory = root.appendingPathComponent("models")
        let decode: @Sendable (URL, Int) throws -> CapturedAudio = { _, limit in
            try expect(limit == 120, "Native audio limit was not forwarded")
            return CapturedAudio(samples: [0.1], sampleRate: 16000, channels: 1)
        }
        let runtime = PhemeRuntime(engine: engine, directory: directory, appRoot: root, decodeAudio: decode)
        let owner = PhemeRuntimeOwner { runtime }
        let contextID = UUID()
        _ = try owner.get(contextID: contextID, reopen: true)
        try await failure("not_ready") { _ = try await runtime.transcribeFile(uri: wav.absoluteString) }
        try await failure("invalid_file") { _ = try await runtime.importModel(uri: "https://example.com/model", name: "test", sha256: hash) }
        try await failure("checksum") { _ = try await runtime.importModel(uri: modelFile.absoluteString, name: "test", sha256: String(repeating: "0", count: 64)) }
        let model = try await runtime.importModel(uri: modelFile.absoluteString, name: "test", sha256: hash)
        try expect(model.sha256 == hash && model.bytes == 3, "Artifact identity was not preserved")
        let models = try await runtime.listModels(); try expect(models.count == 1, "Failed imports leaked into catalog")
        let settings = PhemeSettings(modelID: model.id)
        var automatic = settings; automatic.language = nil
        let encodedSettings = try encodedObject(automatic) as? [String: Any]
        try expect(encodedSettings?["language"] is NSNull, "Automatic language must be explicit null in the JS contract")
        _ = try await runtime.configure(settings)
        for _ in 0..<2 {
            let result = try await runtime.transcribeFile(uri: wav.absoluteString)
            try expect(result.contains("execution_location") && result.contains("local-test-"), "Result correlation missing")
        }
        try expect(engine.loadCount == 1, "Model reloaded per turn")
        let poll = try runtime.pollJSON(); try expect(poll.contains("dropped_events") && poll.contains("batchesJSON"), "Observation contract missing")
        let reopened = PhemeRuntime(engine: FakeEngine(), directory: directory, appRoot: root, decodeAudio: decode)
        let saved = try await reopened.savedSettings(); try expect(saved?.modelID == hash, "Saved setup did not persist")
        let restored = try await reopened.listModels(); try expect(restored.count == 1, "Model catalog did not persist")

        engine.pauseNext()
        let operation = Task { try await runtime.transcribeFile(uri: wav.absoluteString) }
        try await waitUntil { engine.paused }
        try expect(engine.paused, "Fake inference did not start")
        try await failure("busy") { _ = try await runtime.configure(settings) }
        runtime.cancel()
        try expect(!runtime.isSettled && engine.closeCount == 0, "Cancellation released active native work")
        try await failure("busy") { _ = try owner.get(contextID: UUID(), reopen: true) }
        runtime.shutdown()
        try expect(engine.closeCount == 0, "Shutdown closed an active engine")
        engine.pauseClose()
        engine.release()
        try await waitUntil { engine.closeCount == 1 }
        try expect(engine.closeCount == 1 && !runtime.isSettled, "Runtime admitted replacement before model destruction settled")
        engine.releaseClose()
        try await failure("cancelled") { _ = try await operation.value }
        try expect(runtime.isSettled && engine.closeCount == 1, "Engine did not close after settlement")
        try await failure("closed") { _ = try await runtime.transcribeFile(uri: wav.absoluteString) }

        let stored = directory.appendingPathComponent(hash + ".bin")
        try Data("damaged".utf8).write(to: stored)
        try await failure("checksum") { _ = try await reopened.configure(settings) }
        let outside = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try Data("abc".utf8).write(to: outside); defer { try? FileManager.default.removeItem(at: outside) }
        let link = root.appendingPathComponent("escape.bin")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: outside)
        try await failure("invalid_file") { _ = try await reopened.importModel(uri: link.absoluteString, name: "escape", sha256: hash) }
        reopened.shutdown()
        let freshOwner = PhemeRuntimeOwner { PhemeRuntime(engine: FakeEngine(), directory: directory, appRoot: root, decodeAudio: decode) }
        let oldID = UUID(); let newID = UUID()
        let oldRuntime = try freshOwner.get(contextID: oldID, reopen: true)
        let newRuntime = try freshOwner.get(contextID: newID, reopen: true)
        let emptySetup = await freshOwner.respond(contextID: newID) { _ in "null" }
        let emptyObject = try JSONSerialization.jsonObject(with: Data(emptySetup.utf8)) as? [String: Any]
        try expect(emptyObject?["ok"] as? Bool == true && emptyObject?["value"] is NSNull, "First launch cannot represent absent saved settings")
        let encodedFailure = await freshOwner.respond(contextID: newID) { _ in throw BridgeFailure(code: 2, message: "fake engine failure", run_id: "failed-native-1") }
        let failureObject = try JSONSerialization.jsonObject(with: Data(encodedFailure.utf8)) as? [String: Any]
        let detail = failureObject?["error"] as? [String: Any]
        try expect(detail?["code"] as? String == "native_2" && detail?["run_id"] as? String == "failed-native-1", "Native error envelope lost code/run identity")
        freshOwner.close(contextID: oldID)
        try expect(oldRuntime.isClosed && !newRuntime.isClosed, "Old JS context closed the replacement engine")
        try await failure("closed") { _ = try freshOwner.get(contextID: oldID) }
        freshOwner.close(contextID: newID)
        try expect(newRuntime.isClosed, "Current JS context did not close its own engine")
        print("PASS: native runtime import/checksum, catalog/settings persistence, local results, warm reuse, observations, busy/cancel/close settlement, corrupt-model and sandbox recovery")
    }
}
