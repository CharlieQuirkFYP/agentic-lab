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
    @State private var importing = false
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        NavigationStack {
            Form {
                Section("Engine") {
                    LabeledContent("State", value: service.snapshot?.state ?? "starting")
                    if let run = service.snapshot?.active_run { Text("Run: " + run).font(.caption).textSelection(.enabled) }
                    Text(service.message).foregroundStyle(.secondary)
                    if service.busy { ProgressView() }
                }
                Section("Setup") {
                    Picker("Model", selection: $service.setup.modelFile) {
                        Text("Choose a model").tag("")
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
                }.disabled(service.busy || service.recording)
                if let effective = service.snapshot?.configuration {
                    Section("Applied settings") {
                        LabeledContent("Model", value: URL(fileURLWithPath: effective.model_path).lastPathComponent)
                        LabeledContent("Threads", value: String(effective.threads))
                        LabeledContent("Maximum duration", value: "\(effective.max_audio_seconds)s")
                        LabeledContent("Metrics", value: effective.metrics_enabled ? "On" : "Off")
                    }
                }
                Section("Speech") {
                    Button(service.recording ? "Stop and transcribe" : "Record speech") { Task { await service.toggleRecording() } }
                        .disabled(service.busy || service.snapshot?.ready != true)
                    if service.busy { Button("Discard pending result") { service.cancelResult() } }
                    if let result = service.result {
                        Text(result.result.text).textSelection(.enabled)
                        Text("\(result.result.status) · \(Int(result.result.processing_time_ms)) ms").font(.caption)
                        Text(result.run_id).font(.caption2).textSelection(.enabled)
                    }
                }
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
            .task { service.start() }
            .onChange(of: scenePhase) { _, phase in
                if phase != .active {
                    service.interrupt("App inactive. Return to record another turn.")
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
            }
        }
    }
}
