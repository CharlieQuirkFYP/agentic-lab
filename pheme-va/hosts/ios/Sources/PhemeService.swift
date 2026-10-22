import AVFoundation
import Combine
import Foundation

struct SavedSetup: Codable, Sendable {
    var modelFile: String = ""
    var language: String = ""
    var dictionary: String = ""
    var threads: Int = 4
    var maxSeconds: Int = 120
    var metricsEnabled: Bool = true
}

@MainActor
final class PhemeService: ObservableObject {
    @Published var setup = SavedSetup()
    @Published private(set) var models: [String] = []
    @Published private(set) var snapshot: BridgeSnapshot?
    @Published private(set) var observer: ObserverSnapshot?
    @Published private(set) var result: Transcription?
    @Published private(set) var recentBatches: [String] = []
    @Published private(set) var busy = false
    @Published private(set) var recording = false
    @Published private(set) var message = "Choose a local Whisper model to get started."

    private var bridge: NativeBridge?
    private let recorder = AudioRecorder()
    private var poller: Task<Void, Never>?
    private var started = false
    private var generation = UUID()
    private var permissionRequest = UUID()
    private var discardResult = false
    private var unloadWhenIdle = false
    private let bridgeFactory: @Sendable () throws -> NativeBridge
    private let defaults: UserDefaults
    private let directory: URL
    // Hosts can attach one transport to this fan-out, without draining FFI again.
    var onObservation: ((BridgePoll) -> Void)?

    init(defaults: UserDefaults = .standard, directory: URL? = nil, bridgeFactory: @escaping @Sendable () throws -> NativeBridge = { try NativeBridge() }) {
        self.bridgeFactory = bridgeFactory
        self.defaults = defaults
        self.directory = directory ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("PhemeModels", isDirectory: true)
    }

    func start() {
        guard !started else { return }
        started = true
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            refreshModels()
            if let data = defaults.data(forKey: "pheme.setup.v1") {
                do { setup = try JSONDecoder().decode(SavedSetup.self, from: data) }
                catch { message = "Saved setup could not be read. Choose your settings again." }
            }
            try replaceBridge()
            // Unit tests own their fixtures and never load a user's saved model.
            if ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil { return }
            if models.contains(setup.modelFile) { Task { await applySetup() } }
            else if !setup.modelFile.isEmpty { message = "The saved model is missing. Import or select a model." }
        } catch { message = error.localizedDescription }
    }

    func refreshModels() {
        models = ((try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.isRegularFileKey])) ?? [])
            .filter { (try? $0.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true }
            .map(\.lastPathComponent).sorted()
    }

    func importModel(_ url: URL) async {
        guard !busy, !recording else { return }
        busy = true
        message = "Importing model…"
        let destination = directory.appendingPathComponent(UUID().uuidString + "-" + url.lastPathComponent)
        do {
            try await Task.detached(priority: .userInitiated) {
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                try FileManager.default.copyItem(at: url, to: destination)
            }.value
            refreshModels()
            setup.modelFile = destination.lastPathComponent
            message = "Model imported. Apply settings to load it."
        } catch { message = "Import failed: " + error.localizedDescription }
        busy = false
        settle()
    }

    @discardableResult
    func applySetup() async -> Bool {
        guard !busy, !recording else { message = "Finish the current operation first."; return false }
        guard let bridge else { message = "Engine unavailable."; return false }
        // Selection comes from the app-owned directory, never an arbitrary remote path.
        guard models.contains(setup.modelFile) else { message = "Select an imported model."; return false }
        let selected = setup
        let path = directory.appendingPathComponent(selected.modelFile).path
        guard FileManager.default.fileExists(atPath: path) else { refreshModels(); message = "Model is missing."; return false }
        let config = BridgeConfiguration(model_path: path,
            language: selected.language.isEmpty ? nil : selected.language,
            dictionary: selected.dictionary.split(separator: ",").map(String.init),
            threads: selected.threads, max_audio_seconds: selected.maxSeconds,
            metrics_enabled: selected.metricsEnabled)
        busy = true
        var applied = false
        message = "Applying settings…"
        do {
            snapshot = try await bridge.configure(config)
            defaults.set(try JSONEncoder().encode(selected), forKey: "pheme.setup.v1")
            applied = true
            message = "Ready for local transcription."
        } catch {
            message = error.localizedDescription
            snapshot = try? bridge.inspect()
        }
        busy = false
        settle()
        return applied
    }

    func applyRemoteSetup(_ selected: SavedSetup) async -> Bool {
        guard !busy, !recording else { message = "Engine is busy"; return false }
        guard models.contains(selected.modelFile) else { message = "Select an already imported phone model"; return false }
        let old = setup
        setup = selected
        let applied = await applySetup()
        if !applied { setup = old }
        return applied
    }

    func toggleRecording() async {
        if recording { finishRecording(); return }
        guard !busy, snapshot?.ready == true else { message = "Load a model first."; return }
        busy = true
        let request = UUID()
        permissionRequest = request
        let permission = await AVAudioApplication.requestRecordPermission()
        busy = false
        guard permissionRequest == request else { settle(); return }
        guard permission else { message = "Microphone permission is denied. Enable it in Settings."; return }
        guard !unloadWhenIdle else { settle(); return }
        do {
            try recorder.start(maxSeconds: snapshot?.configuration?.max_audio_seconds ?? 120) { [weak self] in self?.finishRecording() }
            recording = true
            message = "Recording. Tap Stop to transcribe."
        } catch { message = error.localizedDescription }
    }

    private func finishRecording() {
        guard recording else { return }
        recording = false
        guard let audio = recorder.stop(), let bridge else { message = "No audio captured."; return }
        busy = true
        discardResult = false
        message = "Transcribing on this iPhone…"
        Task {
            do {
                let transcription = try await bridge.transcribe(audio)
                if !discardResult {
                    result = transcription
                    message = transcription.result.text.isEmpty ? "No speech detected." : "Transcription complete."
                }
            } catch { if !discardResult { message = error.localizedDescription } }
            busy = false
            settle()
        }
    }

    func cancelResult() {
        discardResult = true
        message = "Result discarded. Native work will finish before the model is released."
    }

    func interrupt(_ reason: String) {
        permissionRequest = UUID()
        if recording { _ = recorder.stop(); recording = false }
        discardResult = true
        message = reason
    }

    func memoryWarning() {
        interrupt("Memory pressure: releasing the model when current work finishes.")
        unloadWhenIdle = true
        settle()
    }

    private func settle() {
        guard unloadWhenIdle, !busy else { return }
        unloadWhenIdle = false
        do { try replaceBridge(); message = "Model released. Apply settings when ready." }
        catch { message = error.localizedDescription }
    }

    private func replaceBridge() throws {
        poller?.cancel()
        generation = UUID()
        let currentGeneration = generation
        let owner = try bridgeFactory()
        bridge = owner
        snapshot = try owner.inspect()
        poller = Task { [weak self] in
            while !Task.isCancelled {
                do {
                    let poll = try await Task.detached(priority: .utility) { try owner.poll() }.value
                    guard let self, self.generation == currentGeneration, !Task.isCancelled else { return }
                    self.snapshot = poll.snapshot
                    self.observer = poll.observer
                    if poll.batchesJSON != "[]" {
                        self.recentBatches.append(poll.batchesJSON)
                        while self.recentBatches.count > 20 || self.recentBatches.reduce(0, { $0 + $1.utf8.count }) > 4 * 1024 * 1024 {
                            self.recentBatches.removeFirst()
                        }
                    }
                    self.onObservation?(poll)
                    try await Task.sleep(for: .milliseconds(250))
                } catch is CancellationError { return }
                catch {
                    self?.message = "Observation failed: " + error.localizedDescription
                    try? await Task.sleep(for: .seconds(1))
                }
            }
        }
    }
}
