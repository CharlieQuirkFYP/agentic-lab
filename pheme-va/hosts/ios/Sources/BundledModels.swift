import Foundation

enum BundledModels {
    static var root: URL? {
        guard let url = Bundle.main.resourceURL?.appendingPathComponent("PhemeModels"),
              FileManager.default.fileExists(atPath: url.path) else { return nil }
        return url
    }

    static func files(in directory: URL?, extension suffix: String) -> [String: URL] {
        guard let directory else { return [:] }
        let urls = (try? FileManager.default.contentsOfDirectory(at: directory,
            includingPropertiesForKeys: [.isRegularFileKey])) ?? []
        return Dictionary(uniqueKeysWithValues: urls.filter {
            $0.pathExtension == suffix &&
            (try? $0.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
        }.map { ("Bundled: " + $0.lastPathComponent, $0) })
    }

    static var prompt: String {
        guard let url = root?.appendingPathComponent("incident-reporting.txt"),
              let text = try? String(contentsOf: url, encoding: .utf8) else {
            return "You help review incident descriptions. Ask for missing details, do not invent facts, and do not claim a report has been saved."
        }
        return text
    }
}
