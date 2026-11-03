#if PHEME_REASONING
import Combine
import Foundation

@MainActor
final class ReasoningService: ObservableObject {
    @Published var selectedModel = ""
    @Published var threads = 4
    @Published var draft = ""
    @Published private(set) var models: [String] = []
    @Published private(set) var snapshot: ReasoningSnapshot?
    @Published private(set) var result: ReasoningResult?
    @Published private(set) var busy = false
    @Published private(set) var generating = false
    @Published private(set) var message = "Load the bundled reasoning model to get started."
    private var paths: [String: URL] = [:]
    private var bridge: ReasoningBridge?
    private var started = false
    private var discardResult = false
    private var releaseWhenIdle = false
    private let defaults: UserDefaults
    private let directory: URL?
    private let prompt: String

    var ready: Bool { snapshot?.ready == true && !busy }

    init(defaults: UserDefaults = .standard,
         directory: URL? = BundledModels.root?.appendingPathComponent("Reasoning"),
         prompt: String = BundledModels.prompt) {
        self.defaults = defaults
        self.directory = directory
        self.prompt = prompt
    }

    func start() {
        guard !started else { return }
        started = true
        paths = BundledModels.files(in: directory, extension: "gguf")
        models = paths.keys.sorted()
        let saved = defaults.string(forKey: "pheme.reasoning.model")
        selectedModel = saved.flatMap { paths[$0] != nil ? $0 : nil } ?? models.first ?? ""
        let savedThreads = defaults.integer(forKey: "pheme.reasoning.threads")
        if (1...16).contains(savedThreads) { threads = savedThreads }
        do {
            bridge = try ReasoningBridge()
            snapshot = try bridge?.inspect()
        } catch { message = error.localizedDescription; return }
        guard !selectedModel.isEmpty else {
            message = "No bundled reasoning model. Rebuild using scripts/build-phone.sh."
            return
        }
        if ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] == nil {
            Task { await loadModel() }
        }
    }

    func loadModel() async {
        guard !busy, let path = paths[selectedModel], let bridge else { return }
        busy = true
        message = "Loading reasoning model on this iPhone..."
        result = nil
        do {
            snapshot = try await bridge.load(path: path.path, prompt: prompt, threads: threads)
            defaults.set(selectedModel, forKey: "pheme.reasoning.model")
            defaults.set(threads, forKey: "pheme.reasoning.threads")
            message = "Reasoning model ready. Review text and send."
        } catch {
            snapshot = try? bridge.inspect()
            message = error.localizedDescription
        }
        finish()
    }

    func send() async {
        guard ready, let bridge, !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        let approved = draft
        busy = true
        generating = true
        discardResult = false
        result = nil
        message = "Generating a local reply..."
        do {
            let reply = try await bridge.respond(text: approved)
            if !discardResult { result = reply; message = "Local reply complete." }
        } catch {
            if !discardResult { message = error.localizedDescription }
        }
        finish()
    }

    func cancel(_ reason: String = "Reply cancelled. Waiting for native work to settle.") {
        discardResult = true
        bridge?.cancel()
        message = reason
    }

    func reset() async {
        guard !busy, let bridge else { return }
        busy = true
        do {
            snapshot = try await bridge.reset()
            result = nil
            draft = ""
            message = "Conversation reset. Models remain loaded."
        } catch { message = error.localizedDescription }
        finish()
    }

    func memoryWarning() {
        releaseWhenIdle = true
        cancel("Memory pressure: releasing reasoning weights after current work settles.")
        if !busy { finish() }
    }

    private func finish() {
        busy = false
        generating = false
        if releaseWhenIdle {
            releaseWhenIdle = false
            bridge = nil
            snapshot = nil
            do { bridge = try ReasoningBridge(); snapshot = try bridge?.inspect() }
            catch { message = error.localizedDescription; return }
            message = "Reasoning model released. Tap Load reasoning model to restore it."
        } else { snapshot = try? bridge?.inspect() }
    }
}
#endif
