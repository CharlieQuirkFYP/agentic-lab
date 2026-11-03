import AVFoundation
import SwiftUI
import UniformTypeIdentifiers

@main
struct PhemePhoneApp: App {
    var body: some Scene { WindowGroup { PhemeView() } }
}

struct PhemeView: View {
    @StateObject private var service = PhemeService()
    @StateObject private var monitor = PhoneMonitor()
    #if PHEME_REASONING
    @StateObject private var reasoning = ReasoningService()
    #endif
    @State private var importing = false
    @Environment(\.scenePhase) private var scenePhase

    private var operationBusy: Bool {
        #if PHEME_REASONING
        return service.busy || reasoning.busy
        #else
        return service.busy
        #endif
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Engine") {
                    LabeledContent("ASR state", value: service.snapshot?.state ?? "starting")
                    #if PHEME_REASONING
                    LabeledContent("Reasoning state", value: reasoning.busy ? "busy" : reasoning.snapshot?.state ?? "starting")
                    #endif
                    if let run = service.snapshot?.active_run { Text("Run: " + run).font(.caption).textSelection(.enabled) }
                    Text(service.message).foregroundStyle(.secondary)
                    if service.busy { ProgressView() }
                }
                Section("ASR setup") {
                    Picker("ASR Model (Whisper)", selection: $service.setup.modelFile) {
                        Text("Choose an ASR model").tag("")
                        ForEach(service.models, id: \.self) { Text($0).tag($0) }
                    }
                    Button("Import Whisper model") { importing = true }
                    TextField("Language (blank for automatic)", text: $service.setup.language)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    TextField("Dictionary terms, separated by commas", text: $service.setup.dictionary, axis: .vertical)
                    Stepper("Threads: \(service.setup.threads)", value: $service.setup.threads, in: 1...16)
                    Stepper("Maximum audio: \(service.setup.maxSeconds)s", value: $service.setup.maxSeconds, in: 1...120)
                    Toggle("Collect core metrics", isOn: $service.setup.metricsEnabled)
                    Button("Apply settings and load") { Task { await service.applySetup() } }
                }.disabled(operationBusy || service.recording)
                #if PHEME_REASONING
                Section("Reasoning setup") {
                    Picker("Reasoning Model", selection: $reasoning.selectedModel) {
                        Text("Choose a reasoning model").tag("")
                        ForEach(reasoning.models, id: \.self) { Text($0).tag($0) }
                    }
                    Stepper("Reasoning threads: \(reasoning.threads)", value: $reasoning.threads, in: 1...16)
                    Button("Load reasoning model") { Task { await reasoning.loadModel() } }
                    Text(reasoning.message).font(.caption).foregroundStyle(.secondary)
                    Text("Bundled models run locally. No model download or server is needed on the phone.")
                        .font(.caption).foregroundStyle(.secondary)
                }.disabled(operationBusy || service.recording)
                #endif
                if let effective = service.snapshot?.configuration {
                    Section("Applied settings") {
                        LabeledContent("ASR Model", value: URL(fileURLWithPath: effective.model_path).lastPathComponent)
                        LabeledContent("Threads", value: String(effective.threads))
                        LabeledContent("Maximum duration", value: "\(effective.max_audio_seconds)s")
                        LabeledContent("Metrics", value: effective.metrics_enabled ? "On" : "Off")
                    }
                }
                Section("Speech") {
                    Button(service.recording ? "Stop and transcribe" : "Record speech") { Task { await service.toggleRecording() } }
                        .disabled(operationBusy || service.snapshot?.ready != true)
                    if service.busy { Button("Discard pending result") { service.cancelResult() } }
                    if let result = service.result {
                        Text(result.result.text).textSelection(.enabled)
                        Text("\(result.result.status) · \(Int(result.result.processing_time_ms)) ms").font(.caption)
                        Text(result.run_id).font(.caption2).textSelection(.enabled)
                    }
                }
                #if PHEME_REASONING
                Section("Reviewed text and reply") {
                    Button("Use transcript") {
                        reasoning.draft = service.result?.result.text ?? ""
                    }.disabled(operationBusy || service.recording || service.result?.result.text.isEmpty != false)
                    TextEditor(text: $reasoning.draft).frame(minHeight: 90)
                        .disabled(operationBusy || service.recording)
                        .accessibilityLabel("Reviewed text for reasoning")
                    Button("Send to reasoning model") { Task { await reasoning.send() } }
                        .disabled(operationBusy || service.recording || !reasoning.ready || reasoning.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    if reasoning.generating {
                        ProgressView("Generating local reply")
                        Button("Cancel reply") { reasoning.cancel() }
                    }
                    Button("Reset conversation") { Task { await reasoning.reset() } }
                        .disabled(operationBusy || service.recording)
                    if let result = reasoning.result {
                        Text(result.text).textSelection(.enabled)
                        Text("Reasoning: \(result.processing_time_ms) ms").font(.caption)
                        Text(result.run_id).font(.caption2).textSelection(.enabled)
                    }
                    Text("Replies are development output. This app does not finalize or save incident reports.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                #endif
                Section("Desktop monitor") {
                    TextField("ws://desktop-address:8765", text: $monitor.address)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    SecureField("Pairing code", text: $monitor.token)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    Text(monitor.status).font(.caption)
                    Button(monitor.connected ? "Disconnect monitor" : "Connect monitor") {
                        if monitor.connected { monitor.disconnect() } else { monitor.connect(to: service) }
                    }
                }
                Section("Metrics") {
                    LabeledContent("Dropped events", value: service.observer.map { String($0.dropped_events) } ?? "Unavailable")
                    Text("Core timings and status only. Device power, energy and temperature are not measured.")
                        .font(.caption).foregroundStyle(.secondary)
                    ForEach(Array(service.recentBatches.enumerated()), id: \.offset) { _, batch in
                        DisclosureGroup("Metric batch") { Text(batch).font(.system(.caption, design: .monospaced)).textSelection(.enabled) }
                    }
                }
            }
            .navigationTitle("Pheme")
            .fileImporter(isPresented: $importing, allowedContentTypes: [.data]) { result in
                switch result {
                case .success(let url): Task { await service.importModel(url) }
                case .failure: break
                }
            }
            .task {
                service.start()
                #if PHEME_REASONING
                reasoning.start()
                #endif
            }
            .onChange(of: scenePhase) { _, phase in
                if phase != .active {
                    service.interrupt("App inactive. Return to record another turn.")
                    #if PHEME_REASONING
                    reasoning.cancel("App inactive. Any pending reply is discarded.")
                    #endif
                    monitor.disconnect(reason: "App inactive. Reconnect to resume observation.")
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: AVAudioSession.interruptionNotification)) { _ in
                service.interrupt("Audio interrupted. Tap Record when ready.")
            }
            .onReceive(NotificationCenter.default.publisher(for: AVAudioSession.routeChangeNotification)) { notification in
                guard let raw = notification.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
                      let reason = AVAudioSession.RouteChangeReason(rawValue: raw) else { return }
                // Our own setCategory call also emits a route notification.
                // Only external route changes should stop a new recording.
                if [.oldDeviceUnavailable, .newDeviceAvailable, .override,
                    .noSuitableRouteForCategory, .routeConfigurationChange].contains(reason) {
                    service.interrupt("Audio route changed. Tap Record to use the new route.")
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: UIApplication.didReceiveMemoryWarningNotification)) { _ in
                service.memoryWarning()
                #if PHEME_REASONING
                reasoning.memoryWarning()
                #endif
            }
        }
    }
}
