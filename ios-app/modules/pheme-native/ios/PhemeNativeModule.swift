import ExpoModulesCore
import Foundation
import UIKit

extension NativeBridge: PhemeEngine {}

private enum PhemeHost {
    static let lifecycleQueue = DispatchQueue(label: "org.agenticlab.pheme.lifecycle", qos: .userInitiated)
    static let owner = PhemeRuntimeOwner {
        let directory = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("PhemeModels", isDirectory: true)
        return PhemeRuntime(engine: try NativeBridge(), directory: directory,
            appRoot: URL(fileURLWithPath: NSHomeDirectory()),
            decodeAudio: { url, limit in try PhemeAudio.decode(url, maxSeconds: limit) })
    }
}

public final class PhemeNativeModule: Module {
    private let owner = PhemeHost.owner
    private let contextID = UUID()
    private var memoryWarningObserver: NSObjectProtocol?

    public override func willDestroy() {
        if let observer = memoryWarningObserver { NotificationCenter.default.removeObserver(observer) }
        memoryWarningObserver = nil
        let owner = self.owner; let contextID = self.contextID
        PhemeHost.lifecycleQueue.async { owner.close(contextID: contextID) }
    }

    public func definition() -> ModuleDefinition {
        Name("PhemeNative")
        OnCreate { [weak self, owner, contextID] in
            self?.memoryWarningObserver = NotificationCenter.default.addObserver(forName: UIApplication.didReceiveMemoryWarningNotification, object: nil, queue: .main) { _ in
                PhemeHost.lifecycleQueue.async { owner.close(contextID: contextID) }
            }
        }
        AsyncFunction("open") { [owner, contextID] () async -> String in await owner.respond(contextID: contextID, reopen: true) { try $0.inspectJSON() } }
        AsyncFunction("inspect") { [owner, contextID] () async -> String in await owner.respond(contextID: contextID) { try $0.inspectJSON() } }
        AsyncFunction("listModels") { [owner, contextID] () async -> String in
            await owner.respond(contextID: contextID) { runtime in let models = try await runtime.listModels(); return try json(encodedObject(models)) }
        }
        AsyncFunction("savedSettings") { [owner, contextID] () async -> String in
            await owner.respond(contextID: contextID) { runtime in
                guard let saved = try await runtime.savedSettings() else { return "null" }
                return try json(encodedObject(saved))
            }
        }
        AsyncFunction("importModel") { [owner, contextID] (uri: String, name: String, sha256: String) async -> String in
            await owner.respond(contextID: contextID) { runtime in let model = try await runtime.importModel(uri: uri, name: name, sha256: sha256); return try json(encodedObject(model)) }
        }
        AsyncFunction("configure") { [owner, contextID] (settings: String) async -> String in
            await owner.respond(contextID: contextID) { runtime in
                let selected = try JSONDecoder().decode(PhemeSettings.self, from: Data(settings.utf8))
                let snapshot = try await runtime.configure(selected)
                return try json(encodedObject(snapshot))
            }
        }
        AsyncFunction("transcribeFile") { [owner, contextID] (uri: String) async -> String in
            await owner.respond(contextID: contextID) { try await $0.transcribeFile(uri: uri) }
        }
        AsyncFunction("poll") { [owner, contextID] () async -> String in await owner.respond(contextID: contextID) { try $0.pollJSON() } }
        AsyncFunction("cancel") { [owner, contextID] () -> Void in owner.cancel(contextID: contextID) }
        AsyncFunction("close") { [owner, contextID] () -> Void in owner.close(contextID: contextID) }
        OnAppEntersBackground { [owner, contextID] in PhemeHost.lifecycleQueue.async { owner.cancel(contextID: contextID) } }
        OnAppContextDestroys { [owner, contextID] in PhemeHost.lifecycleQueue.async { owner.close(contextID: contextID) } }
    }
}
