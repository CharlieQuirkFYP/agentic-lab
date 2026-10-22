#if PHEME_DESKTOP_SMOKE
import Foundation
import PhemeVA

@main
struct FFISmoke {
    static func main() async throws {
        precondition(pheme_va_abi_version() == 1)
        let unloaded = try NativeBridge()
        let initial = try unloaded.inspect()
        precondition(initial.state == "unavailable")
        do {
            _ = try await unloaded.configure(BridgeConfiguration(model_path: "/missing/model.bin"))
            fatalError("Missing model unexpectedly loaded")
        } catch let error as BridgeFailure {
            #if PHEME_WHISPER
            precondition(error.code == 2)
            #else
            precondition(error.code == 5)
            #endif
        }
        #if PHEME_WHISPER
        precondition(pheme_va_has_whisper())
        precondition(pheme_va_whisper_new(nil) == nil)
        #else
        precondition(!pheme_va_has_whisper())
        #endif
        #if PHEME_TEST_SUPPORT
        let bridge = try NativeBridge(handle: pheme_va_test_create())
        _ = try await bridge.configure(BridgeConfiguration(model_path: "test-only"))
        let result = try await bridge.transcribe(CapturedAudio(samples: [Float](repeating: 0.05, count: 16000), sampleRate: 16000, channels: 1))
        precondition(result.result.text == "Incident at west entrance.")
        let poll = try bridge.poll()
        let batches = try JSONSerialization.jsonObject(with: Data(poll.batchesJSON.utf8)) as! [[String: Any]]
        precondition(batches.first?["run_id"] as? String == result.run_id)
        let empty = try bridge.poll()
        precondition(empty.batchesJSON == "[]")
        #endif
        print("Swift FFI smoke test passed")
    }
}
#endif
