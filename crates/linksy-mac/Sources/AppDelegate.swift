// The status item and its menu: whether the service runs and which devices are connected,
// each paired device's Send Files, Send Clipboard and Ring, the requests waiting for an
// answer, Pair (a QR code) and Quit.

import AppKit

/// Bigger copied files go as a transfer, as in the apps.
private let maxClipboardFile: Int64 = 10 << 20
private let serviceLabel = "mg.diary.linksy.linksyd"

/// What an Accept or Decline item carries.
private final class Answer: NSObject {
    let request: String
    let accept: Bool
    init(_ request: String, accept: Bool) {
        self.request = request
        self.accept = accept
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private let client = Client()
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let menu = NSMenu()
    private var devices: [Device] = []
    private var requests: [PendingRequest] = []
    private var ringing: Set<String> = []
    private var serviceUp = false
    private var menuOpen = false
    /// Off in tests: opening the menu would wait for a click.
    var opensMenuOnRequest = true
    private var invite: InviteWindow?
    private var refreshSoon: DispatchWorkItem?

    func applicationDidFinishLaunching(_ notification: Notification) {
        menu.delegate = self
        item.menu = menu
        item.button?.imagePosition = .imageLeading
        updateIcon()
        client.onConnect = { [weak self] in
            self?.serviceUp = true
            self?.refresh()
        }
        client.onDisconnect = { [weak self] in
            guard let self else { return }
            serviceUp = false
            devices = []
            requests = []
            ringing = []
            invite?.close()
            updateIcon()
        }
        client.onEvent = { [weak self] event in self?.handle(event) }
        client.start()
        // Forgetting a device or a battery reading send no event of their own
        Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in self?.refresh() }
    }

    // MARK: What's shown

    private func refresh() {
        guard client.isConnected else { return }
        client.call("devices") { [weak self] result in
            guard let self, case .success(let list) = result, let views = list as? [[String: Any]] else { return }
            var devices = views.compactMap(Device.init(view:))
            devices.sort { a, b in
                if a.connected != b.connected { return a.connected }
                return a.name.localizedCaseInsensitiveCompare(b.name) == .orderedAscending
            }
            self.devices = devices
            updateIcon()
        }
    }

    /// Several events come together (a device connecting): refresh once they settle.
    private func refreshAfterEvents() {
        refreshSoon?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.refresh() }
        refreshSoon = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
    }

    private var connectedCount: Int {
        devices.filter(\.connected).count
    }

    private var statusText: String {
        if !serviceUp { return "The Linksy service isn't running" }
        switch connectedCount {
        case 0: return "No devices connected"
        case 1: return "1 device connected"
        case let n: return "\(n) devices connected"
        }
    }

    // Filled in while a device is connected, crossed out while the service is down
    private func updateIcon() {
        let symbol = !serviceUp ? "iphone.slash" : connectedCount > 0 ? "iphone.radiowaves.left.and.right" : "iphone"
        let image = NSImage(systemSymbolName: symbol, accessibilityDescription: "Linksy")
        image?.isTemplate = true
        item.button?.image = image
        item.button?.title = requests.isEmpty ? "" : "\(requests.count)"
        item.button?.toolTip = requests.isEmpty ? statusText : "\(statusText) · \(requests.count) waiting for an answer"
    }

    // MARK: The menu

    func menuWillOpen(_ menu: NSMenu) {
        menuOpen = true
    }

    func menuDidClose(_ menu: NSMenu) {
        menuOpen = false
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        menu.addItem(label(statusText))

        if !requests.isEmpty {
            menu.addItem(.separator())
            for request in requests {
                menu.addItem(requestItem(request))
            }
        }

        menu.addItem(.separator())
        if !serviceUp {
            menu.addItem(action("Start the Service", #selector(startService), symbol: "play"))
        } else if devices.isEmpty {
            menu.addItem(label("No paired devices"))
        } else {
            for device in devices {
                menu.addItem(deviceItem(device))
            }
        }

        menu.addItem(.separator())
        let pair = action("Pair a Device…", #selector(pairDevice), symbol: "qrcode")
        pair.isEnabled = serviceUp
        menu.addItem(pair)

        menu.addItem(.separator())
        let quit = action("Quit Linksy", #selector(quitApp), symbol: nil, key: "q")
        menu.addItem(quit)
        // With Option held: stop the service too (launchd starts it again at the next login)
        let stop = action("Stop the Service and Quit", #selector(stopServiceAndQuit), symbol: nil, key: "q")
        stop.keyEquivalentModifierMask = [.command, .option]
        stop.isAlternate = true
        stop.isEnabled = serviceUp
        menu.addItem(stop)
    }

    private func label(_ title: String) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.isEnabled = false
        return item
    }

    private func action(_ title: String, _ selector: Selector, symbol: String?, key: String = "", with object: Any? = nil) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: selector, keyEquivalent: key)
        item.target = self
        item.representedObject = object
        if let symbol {
            item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
        }
        return item
    }

    private func deviceItem(_ device: Device) -> NSMenuItem {
        let item = NSMenuItem(title: device.name, action: nil, keyEquivalent: "")
        item.image = NSImage(systemSymbolName: device.symbol, accessibilityDescription: nil)
        let sub = NSMenu()
        var state = device.connected ? "Connected" : "Not connected"
        if device.connected, let battery = device.battery {
            state += " · \(battery.level)%\(battery.charging ? ", charging" : "")"
        }
        sub.addItem(label(state))
        sub.addItem(.separator())
        let on = device.connected
        let send = action("Send Files…", #selector(sendFiles), symbol: "paperplane", with: device.id)
        send.isEnabled = on
        sub.addItem(send)
        let clip = action("Send Clipboard", #selector(sendClipboard), symbol: "doc.on.clipboard", with: device.id)
        clip.isEnabled = on
        sub.addItem(clip)
        if device.can("ring") {
            let ringItem = ringing.contains(device.id)
                ? action("Stop Ringing", #selector(stopRinging), symbol: "bell.slash", with: device.id)
                : action("Ring", #selector(ring), symbol: "bell", with: device.id)
            ringItem.isEnabled = on
            sub.addItem(ringItem)
        }
        item.submenu = sub
        return item
    }

    private func requestItem(_ request: PendingRequest) -> NSMenuItem {
        let item = NSMenuItem(title: request.title, action: nil, keyEquivalent: "")
        item.image = NSImage(systemSymbolName: "exclamationmark.circle", accessibilityDescription: nil)
        let sub = NSMenu()
        for line in request.details {
            sub.addItem(label(line))
        }
        if !request.details.isEmpty {
            sub.addItem(.separator())
        }
        sub.addItem(action(request.acceptTitle, #selector(answer), symbol: "checkmark", with: Answer(request.id, accept: true)))
        sub.addItem(action("Decline", #selector(answer), symbol: "xmark", with: Answer(request.id, accept: false)))
        item.submenu = sub
        return item
    }

    // MARK: Events from the service

    func handle(_ event: [String: Any]) {
        guard let type = event["type"] as? String else { return }
        switch type {
        case "pairing_request":
            guard (event["incoming"] as? Bool) == true, let id = event["request"] as? String, let from = Peer(event["device"]), let code = event["code"] as? String else { return }
            add(PendingRequest(id: id, from: from, ask: .pair(code: code)))
        case "transfer_request":
            guard let id = event["transfer"] as? String, let from = Peer(event["from"]) else { return }
            let files = event["files"] as? [[String: Any]] ?? []
            let names = files.compactMap { $0["path"] as? String }
            let total = (event["total"] as? NSNumber)?.int64Value ?? 0
            add(PendingRequest(id: id, from: from, ask: .files(names: names, count: files.count, total: total)))
        case "session_request":
            guard let id = event["request"] as? String, let from = Peer(event["from"]), let kind = event["kind"] as? String else { return }
            add(PendingRequest(id: id, from: from, ask: .session(kind: kind)))
        case "paired":
            guard let device = Peer(event["device"]) else { return }
            drop { if case .pair = $0.ask { return $0.from.id == device.id } else { return false } }
            invite?.paired(with: device.name)
            notify("Linksy", "Paired with \(device.name)")
            refreshAfterEvents()
        case "pairing_failed":
            guard let device = Peer(event["device"]) else { return }
            drop { if case .pair = $0.ask { return $0.from.id == device.id } else { return false } }
            invite?.failed(event["reason"] as? String ?? "")
        case "transfer_started", "transfer_failed":
            if let id = event["transfer"] as? String {
                drop { $0.id == id }
            }
        case "session_started", "session_ended":
            // The request is over either way; the id isn't the request's, so clear by device
            if let peer = Peer(event["peer"]) {
                drop { if case .session = $0.ask { return $0.from.id == peer.id } else { return false } }
            }
        case "ring":
            if let from = Peer(event["from"]), (event["on"] as? Bool) == true {
                notify(from.name, "\(from.name) is looking for this Mac")
            }
        case "device_found", "device_lost", "connected", "disconnected", "device_updated", "battery":
            refreshAfterEvents()
        default:
            break
        }
    }

    /// A new request: on the icon, in Notification Center, and the menu opens so it can be
    /// answered right away.
    private func add(_ request: PendingRequest) {
        requests.removeAll { $0.id == request.id }
        requests.append(request)
        updateIcon()
        notify("Linksy", request.title + " — open the Linksy menu to answer")
        if opensMenuOnRequest && !menuOpen {
            item.button?.performClick(nil)
        }
    }

    private func drop(where gone: (PendingRequest) -> Bool) {
        let before = requests.count
        requests.removeAll(where: gone)
        if requests.count != before {
            updateIcon()
        }
    }

    // MARK: Actions

    private func device(_ sender: NSMenuItem) -> Device? {
        guard let id = sender.representedObject as? String else { return nil }
        return devices.first { $0.id == id }
    }

    @objc private func answer(_ sender: NSMenuItem) {
        guard let answer = sender.representedObject as? Answer else { return }
        drop { $0.id == answer.request }
        client.call("respond", ["request": answer.request, "accept": answer.accept]) { [weak self] result in
            if case .failure(let error) = result {
                self?.report(error, about: "Linksy")
            }
        }
    }

    @objc private func sendFiles(_ sender: NSMenuItem) {
        guard let device = device(sender) else { return }
        let panel = NSOpenPanel()
        panel.title = "Send to \(device.name)"
        panel.message = "Files and folders to send to \(device.name)"
        panel.prompt = "Send"
        panel.canChooseFiles = true
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = true
        panel.level = .floating
        NSApp.activate(ignoringOtherApps: true)
        panel.begin { [weak self] response in
            guard response == .OK, !panel.urls.isEmpty else { return }
            self?.send(panel.urls.map(\.path), to: device)
        }
    }

    private func send(_ paths: [String], to device: Device) {
        client.call("send", ["to": device.id, "paths": paths]) { [weak self] result in
            if case .failure(let error) = result {
                self?.report(error, about: device.name)
            }
        }
    }

    /// As the app's Send Clipboard: one small copied file or an image onto the device's
    /// clipboard, more files as a transfer, else the text.
    @objc private func sendClipboard(_ sender: NSMenuItem) {
        guard let device = device(sender) else { return }
        let pasteboard = NSPasteboard.general
        let done: Client.Reply = { [weak self] result in
            switch result {
            case .success: self?.notify(device.name, "Clipboard sent to \(device.name)")
            case .failure(let error): self?.report(error, about: device.name)
            }
        }
        if let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL], !urls.isEmpty {
            let paths = urls.map(\.path)
            let attributes = paths.count == 1 ? try? FileManager.default.attributesOfItem(atPath: paths[0]) : nil
            let size = (attributes?[.size] as? NSNumber)?.int64Value ?? Int64.max
            let small = (attributes?[.type] as? FileAttributeType) == .typeRegular && size <= maxClipboardFile
            if small && device.can("clipboard_file") {
                client.call("clipboard_file", ["to": device.id, "path": paths[0]], reply: done)
            } else {
                send(paths, to: device)
            }
        } else if let text = pasteboard.string(forType: .string), !text.isEmpty {
            client.call("clipboard", ["to": device.id, "text": text], reply: done)
        } else if let image = NSImage(pasteboard: pasteboard) {
            guard device.can("clipboard_file") else {
                return report(.daemon("\(device.name) can't take images on its clipboard yet"), about: device.name)
            }
            guard let tiff = image.tiffRepresentation, let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) else {
                return report(.daemon("Couldn't read the image on the clipboard"), about: device.name)
            }
            let cache = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("linksy")
            let path = cache.appendingPathComponent("Clipboard image.png")
            do {
                try FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true)
                try png.write(to: path)
            } catch {
                return report(.daemon("Couldn't save the image on the clipboard: \(error.localizedDescription)"), about: device.name)
            }
            client.call("clipboard_file", ["to": device.id, "path": path.path], reply: done)
        } else {
            report(.daemon("There's nothing to send on the clipboard"), about: device.name)
        }
    }

    @objc private func ring(_ sender: NSMenuItem) {
        guard let device = device(sender) else { return }
        client.call("ring", ["to": device.id, "on": true]) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success:
                ringing.insert(device.id)
                // Phones stop by themselves after a while
                DispatchQueue.main.asyncAfter(deadline: .now() + 60) { [weak self] in self?.ringing.remove(device.id) }
            case .failure(let error):
                report(error, about: device.name)
            }
        }
    }

    @objc private func stopRinging(_ sender: NSMenuItem) {
        guard let device = device(sender) else { return }
        ringing.remove(device.id)
        client.call("ring", ["to": device.id, "on": false]) { [weak self] result in
            if case .failure(let error) = result {
                self?.report(error, about: device.name)
            }
        }
    }

    @objc private func pairDevice(_ sender: NSMenuItem) {
        if let invite {
            invite.show()
            return
        }
        client.call("create_invite") { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let uri as String):
                let window = InviteWindow(uri: uri)
                window.onClose = { [weak self] in
                    self?.invite = nil
                    self?.client.call("cancel_invite")
                }
                invite = window
                window.show()
            case .success:
                report(.daemon("The service sent no invite link"), about: "Pair a Device")
            case .failure(let error):
                report(error, about: "Pair a Device")
            }
        }
    }

    /// Ask launchd to start the agent the install script loaded (loading it first if it
    /// isn't).
    @objc private func startService(_ sender: NSMenuItem) {
        let domain = "gui/\(getuid())"
        if run("/bin/launchctl", ["kickstart", "\(domain)/\(serviceLabel)"]) == nil {
            let plist = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/LaunchAgents/\(serviceLabel).plist").path
            if let error = run("/bin/launchctl", ["bootstrap", domain, plist]) {
                report(.daemon("Couldn't start the service: \(error)\nRun scripts/install-macos.sh first."), about: "Linksy")
            }
        }
    }

    @objc private func stopServiceAndQuit(_ sender: NSMenuItem) {
        client.call("quit") { _ in NSApp.terminate(nil) }
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { NSApp.terminate(nil) }
    }

    @objc private func quitApp(_ sender: NSMenuItem) {
        NSApp.terminate(nil)
    }

    // MARK: Telling the user

    /// Nowhere else to say it went wrong: an alert.
    private func report(_ error: ClientError, about what: String) {
        let alert = NSAlert()
        alert.messageText = what
        alert.informativeText = error.description
        alert.alertStyle = .warning
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
    }

    /// A banner in Notification Center, through the system's script runner like the service
    /// does (an unsigned bundle can't post its own).
    private func notify(_ title: String, _ body: String) {
        let script = "display notification \(quoted(body)) with title \(quoted(title))"
        DispatchQueue.global().async {
            _ = self.run("/usr/bin/osascript", ["-e", script])
        }
    }

    private func quoted(_ text: String) -> String {
        var out = "\""
        for c in text {
            switch c {
            case "\\": out += "\\\\"
            case "\"": out += "\\\""
            case "\n": out += "\\n"
            case "\r": out += "\\r"
            case "\t": out += "\\t"
            default: out.append(c)
            }
        }
        return out + "\""
    }

    /// Run a program and wait; its error output when it fails, nil when it worked.
    private func run(_ program: String, _ arguments: [String]) -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: program)
        process.arguments = arguments
        let errors = Pipe()
        process.standardOutput = FileHandle.nullDevice
        process.standardError = errors
        do {
            try process.run()
        } catch {
            return error.localizedDescription
        }
        let output = errors.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        if process.terminationStatus == 0 { return nil }
        let text = String(decoding: output, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? "exit status \(process.terminationStatus)" : text
    }
}
