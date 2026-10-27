import Foundation
import UIKit
import Combine

/// Explicit, paired development transport. The phone always owns inference.
@MainActor
final class PhoneMonitor: ObservableObject {
    @Published var address = ""
    @Published var token = ""
    @Published private(set) var status = "Disconnected"
    @Published private(set) var connected = false
    private var socket: URLSessionWebSocketTask?
    private var receiver: Task<Void, Never>?
    private var writer: Task<Void, Never>?
    private var pendingObservation: String?
    private var replies: [String] = []
    private var generation = UUID()
    private var dropped: UInt64 = 0
    private var cursor: UInt64 = 0
    private var appliedIDs: [String] = []
    private var applying = false
    private let maximumBytes = 256 * 1024

    func connect(to service: PhemeService) {
        disconnect()
        guard let url = URL(string: address), ["ws", "wss"].contains(url.scheme ?? ""),
              url.host != nil, url.user == nil, url.password == nil, token.count == 32 else {
            status = "Enter the desktop WebSocket address and 32-character pairing code."
            return
        }
        let current = generation
        let socket = URLSession.shared.webSocketTask(with: url)
        socket.maximumMessageSize = maximumBytes
        self.socket = socket
        status = "Connecting…"
        service.onObservation = { [weak self, weak service] poll in
            guard let self, let service else { return }
            self.observe(poll, service: service)
        }
        socket.resume()
        receiver = Task { [weak self, weak service] in
            do {
                guard let self else { return }
                let hello: [String: Any] = ["version": 1, "type": "hello", "token": self.token,
                                          "device": String(UIDevice.current.name.prefix(64))]
                try await socket.send(.string(try self.encode(hello)))
                while !Task.isCancelled {
                    let message = try await socket.receive()
                    guard self.generation == current, let service else { return }
                    guard case .string(let text) = message,
                          let value = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any],
                          value["version"] as? Int == 1 else {
                        self.disconnect(reason: "Unsupported desktop message")
                        return
                    }
                    self.receive(value, service: service, generation: current)
                }
            } catch {
                if self?.generation == current { self?.disconnect(reason: error.localizedDescription) }
            }
        }
    }

    func disconnect(reason: String = "Disconnected; phone execution continues") {
        generation = UUID()
        receiver?.cancel()
        writer?.cancel()
        receiver = nil
        writer = nil
        socket?.cancel(with: .goingAway, reason: nil)
        socket = nil
        connected = false
        pendingObservation = nil
        replies = []
        appliedIDs = []
        applying = false
        status = reason
    }

    private func receive(_ value: [String: Any], service: PhemeService, generation: UUID) {
        if value["type"] as? String == "paired" {
            connected = true
            status = "Paired; observing current state"
            return
        }
        guard connected, let id = value["id"] as? String, id.utf8.count <= 128 else {
            disconnect(reason: "Invalid desktop command")
            return
        }
        guard !appliedIDs.contains(id) else { acknowledge(id, accepted: false, message: "Duplicate command"); return }
        appliedIDs.append(id)
        if appliedIDs.count > 128 { appliedIDs.removeFirst() }
        switch value["type"] as? String {
        case "inspect":
            acknowledge(id, accepted: true, message: "Current snapshot follows on the observation stream")
        case "configure":
            guard !applying, !service.busy, !service.recording else {
                acknowledge(id, accepted: false, message: "Engine is busy")
                return
            }
            do {
                guard let settings = value["settings"] else { throw BridgeFailure(code: 1, message: "Missing settings", run_id: nil) }
                let data = try JSONSerialization.data(withJSONObject: settings)
                guard data.count <= 16384 else { throw BridgeFailure(code: 1, message: "Settings exceed 16 KiB", run_id: nil) }
                let selected = try JSONDecoder().decode(SavedSetup.self, from: data)
                applying = true
                Task {
                    let accepted = await service.applyRemoteSetup(selected)
                    // Never replay or acknowledge into a later connection.
                    guard self.generation == generation else { return }
                    self.applying = false
                    self.acknowledge(id, accepted: accepted, message: service.message)
                }
            } catch { acknowledge(id, accepted: false, message: error.localizedDescription) }
        default:
            acknowledge(id, accepted: false, message: "Unsupported command")
        }
    }

    private func observe(_ poll: BridgePoll, service: PhemeService) {
        guard connected else { return }
        do {
            cursor += 1
            var value: [String: Any] = [
                "version": 1, "type": "observation", "cursor": cursor,
                "dropped_observations": dropped, "history_available": false,
                "snapshot": try object(poll.snapshot), "observer": try object(poll.observer),
                "models": service.models, "settings": try object(service.setup),
                "batches_json": poll.batchesJSON
            ]
            if pendingObservation != nil { dropped += 1 }
            var text = try encode(value)
            if text.utf8.count > maximumBytes {
                dropped += 1
                value["batches_json"] = "[]"
            }
            value["dropped_observations"] = dropped
            text = try encode(value)
            guard text.utf8.count <= maximumBytes else {
                disconnect(reason: "Observation exceeds transport limit")
                return
            }
            pendingObservation = text
            pump()
        } catch { disconnect(reason: error.localizedDescription) }
    }

    private func acknowledge(_ id: String, accepted: Bool, message: String) {
        guard replies.count < 8 else { disconnect(reason: "Acknowledgement queue full"); return }
        do {
            replies.append(try encode(["version": 1, "type": "ack", "id": id, "accepted": accepted, "message": message]))
            pump()
        } catch { disconnect(reason: error.localizedDescription) }
    }

    private func pump() {
        guard writer == nil, let socket, connected else { return }
        let current = generation
        writer = Task {
            do {
                while !Task.isCancelled, self.generation == current {
                    let text: String
                    if !self.replies.isEmpty { text = self.replies.removeFirst() }
                    else if let pending = self.pendingObservation { text = pending; self.pendingObservation = nil }
                    else { self.writer = nil; return }
                    try await socket.send(.string(text))
                }
            } catch {
                if self.generation == current { self.disconnect(reason: error.localizedDescription) }
            }
        }
    }

    private func object<T: Encodable>(_ value: T) throws -> Any {
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(value))
    }
    private func encode(_ value: [String: Any]) throws -> String {
        String(decoding: try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]), as: UTF8.self)
    }
}
