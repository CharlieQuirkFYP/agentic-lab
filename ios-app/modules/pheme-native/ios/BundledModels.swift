import Foundation

struct BundledModel: Codable, Sendable {
    let model: PhemeModel
    let purpose: String
    let relativePath: String
}

/// Model resources are read in place, never copied into the phone's data container.
struct BundledModels: Sendable {
    let root: URL?

    static var application: BundledModels {
        // CocoaPods copies this resource bundle beside the Expo application.
        let url = Bundle.main.url(forResource: "PhemeModels", withExtension: "bundle")
        return BundledModels(root: url)
    }

    func entries(purpose: String) throws -> [BundledModel] {
        guard let root else { return [] }
        let entries = try JSONDecoder().decode([BundledModel].self,
            from: Data(contentsOf: root.appendingPathComponent("catalog.json")))
        return entries.filter { $0.purpose == purpose }
    }

    func url(for entry: BundledModel) throws -> URL {
        guard let root else { throw PhemeLocalFailure(code: "missing_model", message: "No bundled models. Rebuild with npm run native:bundle.") }
        let base = root.resolvingSymlinksInPath().standardizedFileURL
        let url = base.appendingPathComponent(entry.relativePath).resolvingSymlinksInPath().standardizedFileURL
        guard url.path.hasPrefix(base.path + "/"),
              (try? url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true else {
            throw PhemeLocalFailure(code: "missing_model", message: "Bundled model path is invalid or missing.")
        }
        return url
    }

    func prompt() throws -> String {
        guard let root else { throw PhemeLocalFailure(code: "missing_model", message: "No bundled incident role.") }
        return try String(contentsOf: root.appendingPathComponent("incident-reporting.txt"), encoding: .utf8)
    }
}
