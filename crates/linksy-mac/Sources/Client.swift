// The service's local IPC, as crates/linksy-ipc does it: newline-delimited JSON over a Unix
// socket. Requests are `{"id": n, "cmd": "...", ...}`, answers `{"id": n, "ok": ...}` or
// `{"id": n, "error": "..."}`, and the service pushes `{"event": {...}}` lines as things
// happen. Replies and events are delivered on the main queue.

import Foundation

enum ClientError: Error, CustomStringConvertible {
    /// The service isn't running, or went away while we waited.
    case disconnected
    /// The service refused: a message for the user.
    case daemon(String)

    var description: String {
        switch self {
        case .disconnected: return "The Linksy service isn't running."
        case .daemon(let message): return message
        }
    }
}

/// Where the service listens: `$LINKSY_SOCKET`, else `linksyd.sock` in `$XDG_RUNTIME_DIR`,
/// else `linksyd-<user>.sock` in the temporary folder (what `linksy_ipc::socket_path` picks
/// on a Mac).
func socketPath() -> String {
    let env = ProcessInfo.processInfo.environment
    if let path = env["LINKSY_SOCKET"], !path.isEmpty { return path }
    if let dir = env["XDG_RUNTIME_DIR"], !dir.isEmpty { return dir + "/linksyd.sock" }
    var tmp = env["TMPDIR"].flatMap { $0.isEmpty ? nil : $0 } ?? NSTemporaryDirectory()
    if !tmp.hasSuffix("/") { tmp += "/" }
    let user = env["USER"].flatMap { $0.isEmpty ? nil : $0 } ?? NSUserName()
    return tmp + "linksyd-\(user).sock"
}

final class Client {
    typealias Reply = (Result<Any, ClientError>) -> Void

    /// Called on the main queue.
    var onConnect: (() -> Void)?
    var onDisconnect: (() -> Void)?
    var onEvent: (([String: Any]) -> Void)?

    let path: String
    private let lock = NSLock()
    private var fd: Int32 = -1
    private var next: UInt64 = 1
    private var pending: [UInt64: Reply] = [:]
    private var retry: DispatchWorkItem?

    init(path: String = socketPath()) {
        self.path = path
    }

    var isConnected: Bool {
        lock.lock(); defer { lock.unlock() }
        return fd >= 0
    }

    /// Connect, and from then on connect again whenever the service goes away.
    func start() {
        connectNow()
    }

    private func connectNow() {
        retry = nil
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return retryLater() }
        var on: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8CString)
        guard bytes.count <= MemoryLayout.size(ofValue: addr.sun_path) else {
            close(fd)
            return retryLater()
        }
        withUnsafeMutableBytes(of: &addr.sun_path) { buffer in
            for (i, b) in bytes.enumerated() { buffer[i] = UInt8(bitPattern: b) }
        }
        let connected = withUnsafePointer(to: &addr) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else {
            close(fd)
            return retryLater()
        }
        lock.lock()
        self.fd = fd
        lock.unlock()
        let thread = Thread { [weak self] in self?.readLoop(fd) }
        thread.name = "linksy-ipc"
        thread.start()
        onConnect?()
    }

    private func retryLater() {
        let work = DispatchWorkItem { [weak self] in self?.connectNow() }
        retry = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 3, execute: work)
    }

    private func readLoop(_ fd: Int32) {
        var buffer = [UInt8](repeating: 0, count: 64 * 1024)
        var line = [UInt8]()
        while true {
            let n = read(fd, &buffer, buffer.count)
            if n <= 0 { break }
            for byte in buffer[0..<n] {
                if byte == UInt8(ascii: "\n") {
                    handle(line)
                    line.removeAll(keepingCapacity: true)
                } else {
                    line.append(byte)
                }
            }
        }
        close(fd)
        lock.lock()
        self.fd = -1
        let waiting = Array(pending.values)
        pending.removeAll()
        lock.unlock()
        DispatchQueue.main.async { [self] in
            for reply in waiting { reply(.failure(.disconnected)) }
            onDisconnect?()
            retryLater()
        }
    }

    private func handle(_ raw: [UInt8]) {
        guard let object = try? JSONSerialization.jsonObject(with: Data(raw)) as? [String: Any] else { return }
        if let id = (object["id"] as? NSNumber)?.uint64Value {
            lock.lock()
            let reply = pending.removeValue(forKey: id)
            lock.unlock()
            guard let reply else { return }
            let result: Result<Any, ClientError>
            if let error = object["error"] as? String {
                result = .failure(.daemon(error))
            } else {
                result = .success(object["ok"] ?? NSNull())
            }
            DispatchQueue.main.async { reply(result) }
        } else if let event = object["event"] as? [String: Any] {
            DispatchQueue.main.async { [self] in onEvent?(event) }
        }
    }

    /// Send `cmd` with its fields; `reply` gets the answer (on the main queue).
    func call(_ cmd: String, _ fields: [String: Any] = [:], reply: Reply? = nil) {
        var message = fields
        message["cmd"] = cmd
        lock.lock()
        let id = next
        next += 1
        message["id"] = id
        let fd = self.fd
        guard fd >= 0, var bytes = try? JSONSerialization.data(withJSONObject: message) else {
            lock.unlock()
            reply?(.failure(.disconnected))
            return
        }
        bytes.append(UInt8(ascii: "\n"))
        if let reply { pending[id] = reply }
        var written = 0
        bytes.withUnsafeBytes { (buffer: UnsafeRawBufferPointer) in
            while written < buffer.count {
                let n = write(fd, buffer.baseAddress! + written, buffer.count - written)
                if n <= 0 { break }
                written += n
            }
        }
        let failed = written < bytes.count
        if failed { pending.removeValue(forKey: id) }
        lock.unlock()
        if failed { reply?(.failure(.disconnected)) }
    }
}
