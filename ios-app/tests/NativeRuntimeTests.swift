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
    func transcribe(_ audio: CapturedAudio, attemptID: String?) async throws -> Transcription {
        let pause = lock.withLock { runs += 1; return shouldPause }
        if pause { await withCheckedContinuation { continuation in lock.withLock { waiter = continuation; shouldPause = false } } }
        return Transcription(schema_version: 1, run_id: attemptID ?? "local-test-\(lock.withLock { runs })",
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

final class FakeReasoning: PhemeReasoningEngine, @unchecked Sendable {
    private let lock = NSLock()
    private var path: String?
    private var waiter: CheckedContinuation<Void, Never>?
    private var history = 0
    private var resets = 0
    private var closes = 0
    var paused: Bool { lock.withLock { waiter != nil } }
    var closeCount: Int { lock.withLock { closes } }
    var resetCount: Int { lock.withLock { resets } }
    func inspect() throws -> ReasoningSnapshot {
        lock.withLock { ReasoningSnapshot(ready: path != nil, state: path == nil ? "unloaded" : "ready",
            model_path: path, threads: 4, active_run: nil, history_turns: history, load_time_ms: 1) }
    }
    func load(path: String, prompt: String, threads: Int) async throws -> ReasoningSnapshot {
        try expect(prompt == "Test incident role", "Bundled role was not passed to Pheme")
        lock.withLock { self.path = path }
        return try inspect()
    }
    func respond(text: String, requestID: String, attemptID: String?) async throws -> ReasoningResult {
        await withCheckedContinuation { continuation in lock.withLock { waiter = continuation } }
        lock.withLock { history += 1 }
        return ReasoningResult(run_id: attemptID ?? requestID, text: "Which entrance?", processing_time_ms: 1)
    }
    func release() { let next = lock.withLock { let value = waiter; waiter = nil; return value }; next?.resume() }
    func reset() async throws -> ReasoningSnapshot {
        lock.withLock { history = 0; resets += 1 }
        return try inspect()
    }
    func cancel() {} // Exercise the native-success / host-cancellation race.
    func close() { lock.withLock { path = nil; closes += 1 } }
}

final class SampleClock: @unchecked Sendable {
    private let lock = NSLock()
    private var time = 1000.0
    private var cpu = 0.0
    func advance(_ milliseconds: Double, cpuSeconds: Double) { lock.withLock { time += milliseconds; cpu += cpuSeconds } }
    func now() -> Double { lock.withLock { time } }
    func reading() -> ResourceReading {
        lock.withLock { ResourceReading(cpuSeconds: cpu, residentBytes: 1024, footprintBytes: 2048,
            batteryLevel: 0, batteryState: "unplugged", thermalState: "fair") }
    }
}

func checkResourceSampler() throws {
    let clock = SampleClock()
    let sampler = NativeResourceSampler(sessionID: "runtime-a", capacity: 3, automatic: false, read: { clock.reading() }, clock: { clock.now() })
    sampler.begin(attempt: NativeAttempt(attempt_id: "attempt-a", stage: "reasoning", native_run_id: "attempt-a"), stage: "reasoning")
    let start = sampler.drain().1
    try expect(start.count == 1 && start[0].values[0].unavailable_reason == "insufficient_cpu_interval", "First CPU sample fabricated a zero")
    struct ServerTimestamp: Decodable { let timestamp_ms: Int64 }
    let timestamps = try JSONDecoder().decode([ServerTimestamp].self, from: JSONEncoder().encode(start))
    try expect(timestamps[0].timestamp_ms > 0 && start[0].timestamp_ms.rounded(.down) == start[0].timestamp_ms,
        "Resource timestamp cannot decode as the server's int64 millisecond contract")
    clock.advance(500, cpuSeconds: 0.75); sampler.tick()
    let measured = sampler.drain().1[0]
    try expect(measured.attempt_id == "attempt-a" && measured.runtime_session_id == "runtime-a" && measured.values[0].number == 150,
        "Process CPU delta lost multi-core accounting or pre-registered identity")
    try expect(measured.values[3].number == 0, "Valid zero battery reading was discarded")
    try expect(measured.values.filter { ["power", "energy", "gpu_utilization"].contains($0.name) }.allSatisfy { $0.number == nil && $0.unavailable_reason != nil }, "Unverified measurements are not explicitly unavailable")
    for _ in 0..<5 { clock.advance(500, cpuSeconds: 0.1); sampler.tick() }
    let overflow = sampler.drain()
    try expect(overflow.0.dropped_samples == 2 && overflow.1.count == 3, "Resource retention is not bounded or dropped samples were hidden")
    sampler.finish(); clock.advance(500, cpuSeconds: 0); sampler.tick()
    let ended = sampler.drain().1
    try expect(ended.last?.attempt_id == nil && ended.last?.stage == "idle", "Idle samples inherited an ended attempt")
    sampler.begin(attempt: NativeAttempt(attempt_id: "attempt-b", stage: "transcription", native_run_id: "attempt-b"), stage: "transcription")
    let next = sampler.drain().1[0]
    try expect(next.attempt_id == "attempt-b" && next.values[0].number == nil, "CPU interval crossed attempt boundaries")
    sampler.setEnabled(false)
    clock.advance(500, cpuSeconds: 0.2); sampler.tick(); sampler.finish()
    try expect(!sampler.status().enabled && sampler.drain().1.isEmpty, "Disabled sampler continued to sample")
    sampler.setEnabled(true)
    clock.advance(500, cpuSeconds: 0.2); sampler.tick()
    let enabledAgain = sampler.drain().1
    try expect(sampler.status().enabled && enabledAgain.count == 1 && enabledAgain[0].values[0].number == nil, "Re-enabled sampler used an interval spanning disabled time")
    sampler.stop(); sampler.tick(); try expect(sampler.drain().1.isEmpty, "Stopped sampler kept collecting")
}

@main struct RuntimeTests {
    static func main() async throws {
        try checkResourceSampler()
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
        let settings = PhemeSettings(modelID: model.id, language: "ms", dictionary: ["KLASS", "Bukit Panjang", "Jalan Buroh"])
        var automatic = settings; automatic.language = nil
        let encodedSettings = try encodedObject(automatic) as? [String: Any]
        try expect(encodedSettings?["language"] is NSNull, "Automatic language must be explicit null in the JS contract")
        let configured = try await runtime.configure(settings)
        try expect(configured.configuration?.language == "ms", "Selected ASR language was not forwarded to BridgeConfiguration")
        try expect(configured.configuration?.dictionary == settings.dictionary, "ASR vocabulary spelling and order changed before reaching BridgeConfiguration")
        for _ in 0..<2 {
            let result = try await runtime.transcribeFile(uri: wav.absoluteString)
            try expect(result.contains("execution_location") && result.contains("local-test-"), "Result correlation missing")
        }
        try expect(engine.loadCount == 1, "Model reloaded per turn")
        let poll = try runtime.pollJSON(); try expect(poll.contains("dropped_events") && poll.contains("batchesJSON"), "Observation contract missing")
        try runtime.setBenchmarkActive(true)
        runtime.suspend()
        let conditions = try JSONSerialization.jsonObject(with: Data(runtime.inspectJSON().utf8)) as! [String: Any]
        try expect(conditions["benchmark_active"] as? Bool == false, "Backgrounding retained benchmark display ownership")
        engine.pauseNext()
        let attributedOperation = Task { try await runtime.transcribeFile(uri: wav.absoluteString, attemptID: "attempt-live") }
        try await waitUntil { engine.paused }
        try await Task.sleep(nanoseconds: 600_000_000)
        let during = try JSONSerialization.jsonObject(with: Data(runtime.pollJSON().utf8)) as! [String: Any]
        try await failure("busy") { try runtime.setResourceSamplingEnabled(false) }
        let liveSamples = during["resource_samples"] as! [[String: Any]]
        try expect(liveSamples.contains { $0["attempt_id"] as? String == "attempt-live" && $0["boundary"] as? String == "periodic" }, "Sampler stopped while native inference was blocked")
        try expect((during["active_attempt"] as? [String: Any])?["native_run_id"] as? String == "attempt-live", "Active attempt was not registered before inference completed")
        engine.release()
        let attributed = try JSONSerialization.jsonObject(with: Data(try await attributedOperation.value.utf8)) as! [String: Any]
        try expect(attributed["attempt_id"] as? String == "attempt-live" && attributed["runtime_session_id"] as? String == runtime.runtimeSessionID, "Result lost attempt/runtime identity")
        let after = try JSONSerialization.jsonObject(with: Data(runtime.pollJSON().utf8)) as! [String: Any]
        try expect((after["resource_samples"] as! [[String: Any]]).contains { $0["attempt_id"] as? String == "attempt-live" && $0["boundary"] as? String == "finish" }, "Final boundary sample lost correlation")
        let failedAttempt = await owner.respond(contextID: contextID) { try await $0.transcribeFile(uri: "https://invalid.local/file", attemptID: "attempt-invalid-file") }
        let failureDetails = (try JSONSerialization.jsonObject(with: Data(failedAttempt.utf8)) as! [String: Any])["error"] as! [String: Any]
        try expect(failureDetails["attempt_id"] as? String == "attempt-invalid-file" && failureDetails["runtime_session_id"] as? String == runtime.runtimeSessionID, "Failure before decoding lost the admitted attempt")
        try runtime.setResourceSamplingEnabled(false)
        let disabledConfig = try JSONSerialization.jsonObject(with: Data(runtime.inspectJSON().utf8)) as! [String: Any]
        try expect((disabledConfig["sampler"] as? [String: Any])?["enabled"] as? Bool == false, "Inspector did not expose applied sampler state")
        let disabledEffective = disabledConfig["effective_configuration"] as! [String: Any]
        try expect((disabledEffective["sampler"] as? [String: Any])?["enabled"] as? Bool == false, "Configuration did not distinguish sampler overhead condition")
        _ = try runtime.pollJSON()
        _ = try await runtime.transcribeFile(uri: wav.absoluteString, attemptID: "without-resource-sampler")
        let withoutResources = try JSONSerialization.jsonObject(with: Data(runtime.pollJSON().utf8)) as! [String: Any]
        try expect((withoutResources["resource_samples"] as? [Any])?.isEmpty == true, "Disabled sampler emitted inference samples")
        try expect((withoutResources["batchesJSON"] as? String)?.contains("local-test-1") == true, "Disabling resources also disabled native inference metrics")
        try runtime.setResourceSamplingEnabled(true)
        let reopened = PhemeRuntime(engine: FakeEngine(), directory: directory, appRoot: root, decodeAudio: decode)
        let saved = try await reopened.savedSettings(); try expect(saved?.modelID == hash, "Saved setup did not persist")
        guard let saved else { throw TestFailure(message: "ASR settings disappeared after reopening") }
        try expect(saved.language == settings.language && saved.dictionary == settings.dictionary, "ASR language or vocabulary did not survive settings restoration")
        let restoredConfiguration = try await reopened.configure(saved).configuration
        try expect(restoredConfiguration?.language == settings.language && restoredConfiguration?.dictionary == settings.dictionary,
            "Restored ASR settings were not forwarded to the new native engine")
        _ = try await reopened.configure(automatic)
        let restoredAutomatic = try await reopened.savedSettings()
        try expect(restoredAutomatic?.language == nil && restoredAutomatic?.dictionary == settings.dictionary,
            "Automatic language or selected vocabulary changed after saving")
        guard let restoredAutomatic else { throw TestFailure(message: "Automatic ASR settings disappeared") }
        let automaticConfiguration = try await reopened.configure(restoredAutomatic).configuration
        try expect(automaticConfiguration?.language == nil && automaticConfiguration?.dictionary == settings.dictionary,
            "Restored automatic-language settings were not forwarded to BridgeConfiguration")
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
        try expect(oldRuntime.runtimeSessionID != newRuntime.runtimeSessionID, "Replacement runtime reused its identity")
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

        let bundle = root.appendingPathComponent("PhemeModels.bundle")
        try FileManager.default.createDirectory(at: bundle, withIntermediateDirectories: true)
        let fixtureModel = PhemeModel(id: hash, name: "Bundled fixture", sha256: hash, bytes: 3)
        let entries = [
            BundledModel(model: fixtureModel, purpose: "transcription", relativePath: "asr.bin"),
            BundledModel(model: fixtureModel, purpose: "reasoning", relativePath: "reply.gguf")
        ]
        try JSONEncoder().encode(entries).write(to: bundle.appendingPathComponent("catalog.json"))
        try Data("abc".utf8).write(to: bundle.appendingPathComponent("asr.bin"))
        try Data("abc".utf8).write(to: bundle.appendingPathComponent("reply.gguf"))
        try Data("Test incident role".utf8).write(to: bundle.appendingPathComponent("incident-reporting.txt"))
        let bundledEngine = FakeEngine(), reasoning = FakeReasoning()
        let imports = root.appendingPathComponent("fresh-imports")
        let bundled = PhemeRuntime(engine: bundledEngine, reasoning: reasoning, bundled: BundledModels(root: bundle),
            directory: imports, appRoot: root, decodeAudio: decode)
        let asrModels = try await bundled.listModels(), replyModels = try await bundled.listReasoningModels()
        try expect(asrModels.count == 1 && replyModels.count == 1, "Bundled model discovery failed")
        _ = try await bundled.configure(PhemeSettings(modelID: hash))
        _ = try await bundled.configureReasoning(modelID: hash, threads: 4)
        let bundledSnapshot = try bundledEngine.inspect()
        try expect(bundledSnapshot.configuration?.model_path == bundle.appendingPathComponent("asr.bin").path, "ASR model was not loaded directly from bundle")
        try expect(!FileManager.default.fileExists(atPath: imports.appendingPathComponent(hash + ".bin").path), "Bundled weights were duplicated")
        let replyOperation = Task { try await bundled.respond(text: "Smoke", requestID: "reply-1") }
        try await waitUntil { reasoning.paused }
        try await failure("busy") { _ = try await bundled.transcribeFile(uri: wav.absoluteString) }
        bundled.cancel()
        reasoning.release()
        try await failure("cancelled") { _ = try await replyOperation.value }
        try expect(reasoning.resetCount == 1, "Discarded native success remained in conversation history")
        let nextReply = Task { try await bundled.respond(text: "Smoke at gate", requestID: "reply-2") }
        try await waitUntil { reasoning.paused }
        bundled.shutdown()
        try expect(reasoning.closeCount == 0 && bundledEngine.closeCount == 0, "Shutdown freed models before reasoning settled")
        reasoning.release()
        try await failure("cancelled") { _ = try await nextReply.value }
        try expect(reasoning.closeCount == 1 && bundledEngine.closeCount == 1, "Both engines must close after settlement")
        print("PASS: native runtime import/checksum, ASR language/dictionary forwarding and restoration, local results, warm reuse, observations, busy/cancel/close settlement, corrupt-model and sandbox recovery")
        print("PASS: bundled ASR/reasoning paths, shared admission, cancellation history and dual-engine lifetime")
    }
}
