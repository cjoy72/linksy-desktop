// What the menu shows, read from the service's JSON.

import Foundation

/// A paired device, from `devices` (one `DeviceView`).
struct Device {
    let id: String
    let name: String
    let kind: String
    let os: String
    let connected: Bool
    let battery: (level: Int, charging: Bool)?
    let features: [String]

    /// A trusted device from one `DeviceView`; nil for a device merely seen on the network.
    init?(view: [String: Any]) {
        guard (view["trusted"] as? Bool) == true, let info = view["device"] as? [String: Any], let id = info["id"] as? String, let name = info["name"] as? String else { return nil }
        self.id = id
        self.name = name
        kind = info["kind"] as? String ?? "phone"
        os = info["os"] as? String ?? ""
        connected = (view["connected"] as? Bool) == true
        features = info["features"] as? [String] ?? []
        if let b = view["battery"] as? [String: Any], let level = (b["level"] as? NSNumber)?.intValue {
            battery = (level, (b["charging"] as? Bool) == true)
        } else {
            battery = nil
        }
    }

    /// Whether it can do something: what it says, or for an older phone that says nothing,
    /// all but what came later (as the apps and the tray decide).
    func can(_ feature: String) -> Bool {
        if !features.isEmpty { return features.contains(feature) }
        return !(os == "android" && ["microphone", "locate"].contains(feature))
    }

    /// An SF Symbol for the kind of device.
    var symbol: String {
        switch kind {
        case "tablet": return "ipad"
        case "laptop": return "laptopcomputer"
        case "desktop": return "desktopcomputer"
        default: return "iphone"
        }
    }
}

/// The other side of a request: enough of a `DeviceInfo` to name it.
struct Peer {
    let id: String
    let name: String

    init?(_ info: Any?) {
        guard let info = info as? [String: Any], let id = info["id"] as? String, let name = info["name"] as? String else { return nil }
        self.id = id
        self.name = name
    }
}

/// Something a device asks that the user must answer (`respond`), from an event.
struct PendingRequest {
    enum Ask {
        /// Both screens show `code`; accept when they match.
        case pair(code: String)
        case files(names: [String], count: Int, total: Int64)
        /// A media session, by its `SessionKind` name ("screen", "camera"…).
        case session(kind: String)
    }

    let id: String
    let from: Peer
    let ask: Ask
    let at = Date()

    /// The menu entry's title: who wants what.
    var title: String {
        switch ask {
        case .pair:
            return "\(from.name) wants to pair"
        case .files(_, let count, let total):
            let size = ByteCountFormatter.string(fromByteCount: total, countStyle: .file)
            return "\(from.name) wants to send \(count == 1 ? "1 item" : "\(count) items") (\(size))"
        case .session(let kind):
            switch kind {
            case "control": return "\(from.name) wants to control your screen"
            case "screen": return "\(from.name) wants to see your screen"
            case "display": return "\(from.name) wants to use this screen as a second display"
            case "camera": return "\(from.name) wants to use your camera"
            case "input": return "\(from.name) wants to use your mouse and keyboard"
            case "microphone": return "\(from.name) wants to use your microphone"
            case "terminal": return "\(from.name) wants a terminal on this computer"
            default: return "\(from.name) wants to start a \(kind) session"
            }
        }
    }

    /// The lines shown above Accept and Decline.
    var details: [String] {
        switch ask {
        case .pair(let code):
            let spaced = code.count == 6 ? code.prefix(3) + " " + code.suffix(3) : Substring(code)
            return ["Code \(spaced): accept if the other screen shows the same"]
        case .files(let names, let count, _):
            var lines = Array(names.prefix(5))
            if count > 5 { lines.append("…and \(count - 5) more") }
            return lines
        case .session:
            return []
        }
    }

    var acceptTitle: String {
        if case .session = ask { return "Allow" }
        return "Accept"
    }
}
