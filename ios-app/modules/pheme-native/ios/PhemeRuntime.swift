import Foundation
import CryptoKit

struct PhemeLocalFailure: Error, LocalizedError {
    let code: String
    let message: String
    var errorDescription: String? { message }
}

struct PhemeModel: Codable, Sendable {
    let id: String
    let name: String
    let sha256: String
    let bytes: UInt64
}

struct PhemeSettings: Codable, Sendable {
    let modelID: String
    var language: String? = "en"
    var dictionary: [String] = []
    var threads: Int = 4
    var maxAudioSeconds: Int = 120
    var metricsEnabled: Bool = true

    enum CodingKeys: String, CodingKey { case modelID, language, dictionary, threads, maxAudioSeconds, metricsEnabled }
    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(modelID, forKey: .modelID)
        if let language { try container.encode(language, forKey: .language) }
        else { try container.encodeNil(forKey: .language) }
        try container.encode(dictionary, forKey: .dictionary)
        try container.encode(threads, forKey: .threads)
        try container.encode(maxAudioSeconds, forKey: .maxAudioSeconds)
        try container.encode(metricsEnabled, forKey: .metricsEnabled)
    }
}

protocol PhemeEngine: Sendable {
    func configure(_ configuration: BridgeConfiguration) async throws -> BridgeSnapshot
    func transcribe(_ audio: CapturedAudio) async throws -> Transcription
    func inspect() throws -> BridgeSnapshot
    func poll() throws -> BridgePoll
    func close() throws
}

/// App-owned admission/lifetime wrapper. Engine work retains this runtime even
/// when JavaScript reloads or its promise is abandoned. No network calls exist here.
final class PhemeRuntime: @unchecked Sendable {
    private let engine: any PhemeEngine
    private let directory: URL
    private let appRoot: URL
    private let decodeAudio: @Sendable (URL, Int) throws -> CapturedAudio
    private let lock = NSLock()
    private let observerLock = NSLock()
    private let fileQueue = DispatchQueue(label: "org.agenticlab.pheme.files", qos: .userInitiated)
    private var busy = false
    private var closed = false
    private var generation = 0
    private var currentModel: PhemeModel?
    private var settings: PhemeSettings?

    init(engine: any PhemeEngine, directory: URL, appRoot: URL,
         decodeAudio: @escaping @Sendable (URL, Int) throws -> CapturedAudio) {
        self.engine = engine
        self.directory = directory
        self.appRoot = appRoot.resolvingSymlinksInPath().standardizedFileURL
        self.decodeAudio = decodeAudio
    }

    deinit { shutdown() }

    var isClosed: Bool { lock.withLock { closed } }
    var isSettled: Bool { lock.withLock { !busy } }

    private func admit() throws -> Int {
        try lock.withLock {
            guard !closed else { throw PhemeLocalFailure(code: "closed", message: "Reopen the local screen to create a new engine.") }
            guard !busy else { throw PhemeLocalFailure(code: "busy", message: "Wait for the active native operation to settle.") }
            busy = true
            return generation
        }
    }

    private func finish() {
        let shouldClose = lock.withLock { () -> Bool in
            if closed { return true } // Stay busy until model destruction settles.
            busy = false
            return false
        }
        if shouldClose { try? engine.close(); lock.withLock { busy = false } }
    }

    func cancel() {
        lock.withLock { generation += 1 }
    }

    func shutdown() {
        let shouldClose = lock.withLock { () -> Bool in
            if closed { return false }
            closed = true
            generation += 1
            if busy { return false }
            busy = true
            return true
        }
        if shouldClose { try? engine.close(); lock.withLock { busy = false } }
    }

    private func checkGeneration(_ admitted: Int) throws {
        let discarded = lock.withLock { closed || generation != admitted }
        if discarded { throw PhemeLocalFailure(code: "cancelled", message: "Result discarded. Native work has now settled.") }
    }

    private func fileWork<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            fileQueue.async { continuation.resume(with: Result(catching: work)) }
        }
    }

    private func localFile(_ uri: String) throws -> URL {
        guard let url = URL(string: uri), url.isFileURL,
              url.host == nil || url.host == "" || url.host == "localhost" else {
            throw PhemeLocalFailure(code: "invalid_file", message: "Choose a file copied into this app's storage.")
        }
        let resolved = url.resolvingSymlinksInPath().standardizedFileURL
        guard resolved.path.hasPrefix(appRoot.path + "/"),
              (try? resolved.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true else {
            throw PhemeLocalFailure(code: "invalid_file", message: "File is missing or outside this app's storage.")
        }
        return resolved
    }

    private func catalog() throws -> [PhemeModel] {
        let path = directory.appendingPathComponent("models.json")
        if !FileManager.default.fileExists(atPath: path.path) { return [] }
        return try JSONDecoder().decode([PhemeModel].self, from: Data(contentsOf: path))
    }

    func listModels() async throws -> [PhemeModel] {
        _ = try admit(); defer { finish() }
        return try await fileWork { [self] in
            try catalog().filter { FileManager.default.fileExists(atPath: directory.appendingPathComponent($0.id + ".bin").path) }
        }
    }

    func savedSettings() async throws -> PhemeSettings? {
        _ = try admit(); defer { finish() }
        return try await fileWork { [self] in
            let file = directory.appendingPathComponent("settings.json")
            if !FileManager.default.fileExists(atPath: file.path) { return nil }
            return try JSONDecoder().decode(PhemeSettings.self, from: Data(contentsOf: file))
        }
    }

    func importModel(uri: String, name: String, sha256: String) async throws -> PhemeModel {
        let admitted = try admit(); defer { finish() }
        let expected = sha256.lowercased()
        guard expected.count == 64, expected.allSatisfy({ "0123456789abcdef".contains($0) }),
              !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, name.utf8.count <= 256 else {
            throw PhemeLocalFailure(code: "invalid_model", message: "Supply a model name and its trusted 64-character SHA-256 checksum.")
        }
        let imported = try await fileWork { [self] in
            let inputURL = try localFile(uri)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let temporary = directory.appendingPathComponent(UUID().uuidString + ".part")
            defer { try? FileManager.default.removeItem(at: temporary) }
            guard FileManager.default.createFile(atPath: temporary.path, contents: nil) else {
                throw PhemeLocalFailure(code: "storage", message: "Could not create the imported model file.")
            }
            let input = try FileHandle(forReadingFrom: inputURL)
            let output = try FileHandle(forWritingTo: temporary)
            defer { try? input.close(); try? output.close() }
            var hash = SHA256(); var bytes: UInt64 = 0
            while let chunk = try input.read(upToCount: 1024 * 1024), !chunk.isEmpty {
                try checkGeneration(admitted)
                bytes += UInt64(chunk.count)
                guard bytes <= 8 * 1024 * 1024 * 1024 else { throw PhemeLocalFailure(code: "model_too_large", message: "Model exceeds the 8 GiB import limit.") }
                hash.update(data: chunk)
                try output.write(contentsOf: chunk)
            }
            try output.synchronize()
            let actual = hash.finalize().map { String(format: "%02x", $0) }.joined()
            guard bytes > 0, actual == expected else { throw PhemeLocalFailure(code: "checksum", message: "Model checksum does not match the trusted artifact.") }
            let destination = directory.appendingPathComponent(actual + ".bin")
            if FileManager.default.fileExists(atPath: destination.path) { try FileManager.default.removeItem(at: destination) }
            try FileManager.default.moveItem(at: temporary, to: destination)
            let model = PhemeModel(id: actual, name: name, sha256: actual, bytes: bytes)
            var models = try catalog().filter { $0.id != actual }; models.append(model)
            try JSONEncoder().encode(models).write(to: directory.appendingPathComponent("models.json"), options: .atomic)
            return model
        }
        try checkGeneration(admitted)
        return imported
    }

    func configure(_ selected: PhemeSettings) async throws -> BridgeSnapshot {
        let admitted = try admit(); defer { finish() }
        guard selected.modelID.count == 64, selected.modelID.allSatisfy({ "0123456789abcdef".contains($0) }),
              (1...16).contains(selected.threads), (1...120).contains(selected.maxAudioSeconds) else {
            throw PhemeLocalFailure(code: "invalid_configuration", message: "Choose an imported model, 1–16 threads and an audio limit of 1–120 seconds.")
        }
        let model = try await fileWork { [self] in
            guard let model = try catalog().first(where: { $0.id == selected.modelID }),
                  FileManager.default.fileExists(atPath: directory.appendingPathComponent(model.id + ".bin").path) else {
                throw PhemeLocalFailure(code: "missing_model", message: "Import the selected model again.")
            }
            let input = try FileHandle(forReadingFrom: directory.appendingPathComponent(model.id + ".bin"))
            defer { try? input.close() }
            var hash = SHA256()
            while let chunk = try input.read(upToCount: 1024 * 1024), !chunk.isEmpty { try checkGeneration(admitted); hash.update(data: chunk) }
            guard hash.finalize().map({ String(format: "%02x", $0) }).joined() == model.sha256 else {
                throw PhemeLocalFailure(code: "checksum", message: "Stored model is damaged. Import the trusted artifact again.")
            }
            return model
        }
        let configuration = BridgeConfiguration(model_path: directory.appendingPathComponent(model.id + ".bin").path,
            language: selected.language, dictionary: selected.dictionary, threads: selected.threads,
            max_audio_seconds: selected.maxAudioSeconds, metrics_enabled: selected.metricsEnabled)
        try checkGeneration(admitted)
        let snapshot = try await engine.configure(configuration)
        try checkGeneration(admitted)
        lock.withLock { currentModel = model; settings = selected }
        try await fileWork { [self] in
            try JSONEncoder().encode(selected).write(to: directory.appendingPathComponent("settings.json"), options: .atomic)
        }
        try checkGeneration(admitted)
        return snapshot
    }

    func transcribeFile(uri: String) async throws -> String {
        let admitted = try admit(); defer { finish() }
        let selection = lock.withLock { (currentModel, settings) }
        guard let model = selection.0, let settings = selection.1, try engine.inspect().ready else {
            throw PhemeLocalFailure(code: "not_ready", message: "Load a local model first.")
        }
        let audio = try await fileWork { [self] in try decodeAudio(localFile(uri), settings.maxAudioSeconds) }
        try checkGeneration(admitted)
        let result = try await engine.transcribe(audio)
        try checkGeneration(admitted)
        // Return the versioned transcription contract plus verified artifact identity.
        return try json(["transcription": encodedObject(result), "model": encodedObject(model), "settings": encodedObject(settings), "execution_location": "device"])
    }

    func inspectJSON() throws -> String {
        let state = lock.withLock { (busy, closed, currentModel, settings) }
        return try json(["snapshot": encodedObject(engine.inspect()), "busy": state.0, "closed": state.1,
                         "model": try state.2.map(encodedObject) ?? NSNull(), "settings": try state.3.map(encodedObject) ?? NSNull()])
    }

    func pollJSON() throws -> String {
        observerLock.lock(); defer { observerLock.unlock() }
        guard !lock.withLock({ closed }) else { throw PhemeLocalFailure(code: "closed", message: "Local engine is closed.") }
        let poll = try engine.poll()
        return try json(["snapshot": encodedObject(poll.snapshot), "observer": encodedObject(poll.observer), "batchesJSON": poll.batchesJSON])
    }
}

func encodedObject<T: Encodable>(_ value: T) throws -> Any {
    try JSONSerialization.jsonObject(with: JSONEncoder().encode(value))
}

func json(_ value: Any) throws -> String {
    String(decoding: try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]), as: UTF8.self)
}

/// Process-wide engine owner. Late destruction/polls from an older JS context
/// cannot close or silently reopen a newer context's engine.
final class PhemeRuntimeOwner: @unchecked Sendable {
    private let lock = NSLock()
    private let factory: @Sendable () throws -> PhemeRuntime
    private var current: PhemeRuntime?
    private var context: UUID?

    init(factory: @escaping @Sendable () throws -> PhemeRuntime) { self.factory = factory }

    func get(contextID: UUID, reopen: Bool = false) throws -> PhemeRuntime {
        try lock.withLock {
            if let current {
                if context == contextID && (!current.isClosed || !reopen) { return current }
                guard reopen else { throw PhemeLocalFailure(code: "closed", message: "This JavaScript context no longer owns the local engine.") }
                current.shutdown()
                guard current.isSettled else { throw PhemeLocalFailure(code: "busy", message: "Previous native work must settle before opening another engine.") }
            } else if !reopen {
                throw PhemeLocalFailure(code: "not_open", message: "Open the local engine first.")
            }
            let next = try factory()
            current = next; context = contextID
            return next
        }
    }

    func cancel(contextID: UUID) { lock.withLock { if context == contextID { current?.cancel() } } }
    func close(contextID: UUID) { lock.withLock { if context == contextID { current?.shutdown() } } }

    func respond(contextID: UUID, reopen: Bool = false, _ work: @Sendable (PhemeRuntime) async throws -> String) async -> String {
        do {
            let payload = try await work(get(contextID: contextID, reopen: reopen))
            return try json(["ok": true, "value": JSONSerialization.jsonObject(with: Data(payload.utf8), options: [.fragmentsAllowed])])
        } catch {
            var code = "native_failure"
            var runID: String?
            var nativeCode: Int32?
            if let local = error as? PhemeLocalFailure { code = local.code }
            if let failure = error as? BridgeFailure { code = "native_\(failure.code)"; runID = failure.run_id; nativeCode = failure.code }
            return (try? json(["ok": false, "error": ["code": code, "message": error.localizedDescription,
                "run_id": runID as Any? ?? NSNull(), "native_code": nativeCode as Any? ?? NSNull()]]))
                ?? "{\"ok\":false,\"error\":{\"code\":\"serialization\",\"message\":\"Native error could not be encoded.\"}}"
        }
    }
}
