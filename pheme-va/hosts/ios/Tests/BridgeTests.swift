import AVFoundation
import PhemeVA
import XCTest
@testable import PhemePhone

final class BridgeTests: XCTestCase {
    func testCapabilitiesAndUnloadedStatus() throws {
        XCTAssertEqual(pheme_va_abi_version(), 1)
        #if PHEME_WHISPER
        XCTAssertTrue(pheme_va_has_whisper())
        #else
        XCTAssertFalse(pheme_va_has_whisper())
        #endif
        let bridge = try NativeBridge()
        let snapshot = try bridge.inspect()
        XCTAssertEqual(snapshot.state, "unavailable")
        XCTAssertFalse(snapshot.ready)
        XCTAssertNil(snapshot.configuration)
        XCTAssertEqual(try bridge.poll().batchesJSON, "[]")
    }

    func testInvalidConfigurationAndModelFailure() async throws {
        let bridge = try NativeBridge()
        do {
            _ = try await bridge.configure(BridgeConfiguration(model_path: "missing", threads: 0))
            XCTFail("Invalid threads accepted")
        } catch let error as BridgeFailure { XCTAssertEqual(error.code, 1) }
        do {
            _ = try await bridge.configure(BridgeConfiguration(model_path: "/missing/pheme-model.bin"))
            XCTFail("Missing model loaded")
        } catch let error as BridgeFailure {
            #if PHEME_WHISPER
            XCTAssertEqual(error.code, 2)
            #else
            XCTAssertEqual(error.code, 5)
            #endif
        }
        let status = try bridge.inspect()
        XCTAssertEqual(status.state, "unavailable")
        XCTAssertNil(status.configuration)
        XCTAssertNotNil(status.last_failure)
    }

    func testNullArgumentsAndStringOwnership() {
        var output: UnsafeMutablePointer<CChar>?
        XCTAssertEqual(pheme_va_inspect(nil, &output), 1)
        XCTAssertNotNil(output)
        pheme_va_string_free(output)
        pheme_va_string_free(nil)
        pheme_va_free(nil)
        pheme_va_observer_free(nil)
        #if PHEME_WHISPER
        XCTAssertNil(pheme_va_whisper_new(nil))
        XCTAssertNil("/missing/pheme.bin".withCString { pheme_va_whisper_new($0) })
        XCTAssertEqual(pheme_va_whisper_set_dictionary(nil, nil), 1)
        output = nil
        XCTAssertEqual(pheme_va_transcribe(nil, nil, 0, 16000, 1, &output), 1)
        XCTAssertNil(output)
        XCTAssertEqual(pheme_va_metrics_set_enabled(nil, true), 1)
        XCTAssertEqual(pheme_va_metrics_drain(nil, &output), 1)
        pheme_va_string_free(output)
        pheme_va_whisper_free(nil)
        #endif
    }

    #if PHEME_TEST_SUPPORT
    func testFakeSuccessfulCoreOperationAndRunCorrelation() async throws {
        let bridge = try NativeBridge(handle: pheme_va_test_new())
        let audio = CapturedAudio(samples: [Float](repeating: 0.05, count: 16000), sampleRate: 16000, channels: 1)
        for _ in 0..<3 {
            let result = try await bridge.transcribe(audio)
            XCTAssertEqual(result.result.text, "Incident at west entrance.")
            let poll = try bridge.poll()
            let batches = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(poll.batchesJSON.utf8)) as? [[String: Any]])
            XCTAssertEqual(batches.first?["run_id"] as? String, result.run_id)
            XCTAssertEqual(try bridge.poll().batchesJSON, "[]")
        }
        var config = try XCTUnwrap(bridge.inspect().configuration)
        config.threads = 7
        config.metrics_enabled = false
        let applied = try await bridge.configure(config)
        XCTAssertEqual(applied.configuration?.threads, 7)
        _ = try await bridge.transcribe(audio)
        XCTAssertEqual(try bridge.poll().batchesJSON, "[]")
    }

    @MainActor
    func testRemoteSetupAppliesToUnconfiguredServiceAndPersistsOnlySuccess() async throws {
        let suite = "pheme-remote-tests-" + UUID().uuidString
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(suite)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data("fake fixture".utf8).write(to: directory.appendingPathComponent("fake.bin"))
        defer { try? FileManager.default.removeItem(at: directory) }
        let service = PhemeService(defaults: defaults, directory: directory,
            bridgeFactory: { try NativeBridge(handle: pheme_va_test_create()) })
        service.start()
        XCTAssertFalse(service.snapshot?.ready ?? true)
        var selected = SavedSetup()
        selected.modelFile = "fake.bin"
        selected.threads = 6
        let accepted = await service.applyRemoteSetup(selected)
        XCTAssertTrue(accepted)
        let saved = try XCTUnwrap(defaults.data(forKey: "pheme.setup.v1"))
        XCTAssertEqual(try JSONDecoder().decode(SavedSetup.self, from: saved).threads, 6)
        selected.threads = 0
        let rejected = await service.applyRemoteSetup(selected)
        XCTAssertFalse(rejected)
        XCTAssertEqual(defaults.data(forKey: "pheme.setup.v1"), saved)
        selected.threads = 4
        selected.modelFile = "../outside.bin"
        let outside = await service.applyRemoteSetup(selected)
        XCTAssertFalse(outside)
        service.memoryWarning()
        XCTAssertFalse(service.snapshot?.ready ?? true)
        XCTAssertEqual(defaults.data(forKey: "pheme.setup.v1"), saved)
    }

    func testObserverRetainsOutputAfterEngineFree() throws {
        let engine = pheme_va_test_new()
        let observer = pheme_va_observer_new(engine)
        pheme_va_free(engine)
        var output: UnsafeMutablePointer<CChar>?
        XCTAssertEqual(pheme_va_observer_drain(observer, &output), 0)
        XCTAssertEqual(output.map { String(cString: $0) }, "[]")
        pheme_va_string_free(output)
        pheme_va_observer_free(observer)
    }
    #endif

    #if PHEME_WHISPER
    func testOptionalRealTranscription() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard let modelPath = environment["PHEME_VA_TEST_MODEL"], !modelPath.isEmpty,
              let audioPath = environment["PHEME_VA_TEST_AUDIO"], !audioPath.isEmpty else {
            throw XCTSkip("Manual model test was not requested")
        }
        func fixture(_ path: String) -> URL {
            // CI copies fixtures into this test bundle; explicit local paths also work.
            let bundled = Bundle(for: BridgeTests.self).resourceURL!
                .appendingPathComponent("assets").appendingPathComponent(URL(fileURLWithPath: path).lastPathComponent)
            return FileManager.default.fileExists(atPath: bundled.path) ? bundled : URL(fileURLWithPath: path)
        }
        let bridge = try NativeBridge()
        _ = try await bridge.configure(BridgeConfiguration(model_path: fixture(modelPath).path, language: "en"))
        let file = try AVAudioFile(forReading: fixture(audioPath), commonFormat: .pcmFormatFloat32, interleaved: false)
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)))
        try file.read(into: buffer)
        let data = try XCTUnwrap(buffer.floatChannelData)
        let channels = Int(buffer.format.channelCount)
        var samples: [Float] = []
        for frame in 0..<Int(buffer.frameLength) {
            for channel in 0..<channels { samples.append(data[channel][frame]) }
        }
        let result = try await bridge.transcribe(CapturedAudio(samples: samples,
            sampleRate: UInt32(buffer.format.sampleRate), channels: UInt16(channels)))
        XCTAssertEqual(result.result.status, "Speech")
        XCTAssertTrue(result.result.text.lowercased().contains("country"), result.result.text)
        XCTAssertFalse(try bridge.poll().batchesJSON.isEmpty)
    }
    #endif

    @MainActor
    func testSavedSetupRestoresWithoutLoadingUnavailableModel() throws {
        let suite = "pheme-tests-" + UUID().uuidString
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        var saved = SavedSetup()
        saved.modelFile = "missing.bin"
        saved.threads = 3
        defaults.set(try JSONEncoder().encode(saved), forKey: "pheme.setup.v1")
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(suite)
        defer { try? FileManager.default.removeItem(at: directory) }
        let service = PhemeService(defaults: defaults, directory: directory)
        service.start()
        XCTAssertEqual(service.setup.threads, 3)
        XCTAssertFalse(service.snapshot?.ready ?? true)
    }
}
