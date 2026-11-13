import Darwin
import Foundation
import LauncherCore

enum WorkerError: Error, LocalizedError {
    case programMissing
    case exited(Int32)
    case timeout

    var errorDescription: String? {
        switch self {
        case .programMissing:
            return L("Das Programm reco-minimap fehlt in dieser Version. Bitte Reco Minimap neu installieren.",
                     "The reco-minimap program is missing from this version. Please reinstall Reco Minimap.")
        case .exited(let status):
            return L("reco-minimap wurde unerwartet beendet (Status \(status)). Details stehen im Protokoll.",
                     "reco-minimap exited unexpectedly (status \(status)). The log has the details.")
        case .timeout:
            return L("reco-minimap hat sich nicht rechtzeitig gemeldet.", "reco-minimap did not respond in time.")
        }
    }
}

/// Starts and supervises `reco-minimap ui`, which serves the web interface and runs every job as
/// a child process of its own.
final class WorkerController {
    static let preferredPort = 8788

    private(set) var port = WorkerController.preferredPort
    private var process: Process?

    var baseURL: URL { URL(string: "http://127.0.0.1:\(port)")! }
    var isRunning: Bool { process?.isRunning == true }

    static var logURL: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/Reco Minimap/reco-minimap.log")
    }

    func start(payload: Payload) async throws {
        stop()
        guard FileManager.default.isExecutableFile(atPath: payload.executable.path) else { throw WorkerError.programMissing }
        port = Self.firstFreePort(startingAt: Self.preferredPort)

        let logURL = Self.logURL
        try FileManager.default.createDirectory(at: logURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        if let size = (try? FileManager.default.attributesOfItem(atPath: logURL.path)[.size]) as? Int, size > 5_000_000 {
            try? FileManager.default.removeItem(at: logURL)
        }
        if !FileManager.default.fileExists(atPath: logURL.path) {
            FileManager.default.createFile(atPath: logURL.path, contents: nil)
        }
        let log = try FileHandle(forWritingTo: logURL)
        try log.seekToEnd()

        let process = Process()
        process.executableURL = payload.executable
        // the launcher installs updates itself, so the program's own update check stays off
        process.arguments = ["ui", "--no-open", "--no-update-check", "--port", String(port)]
        process.currentDirectoryURL = payload.directory
        var environment = ProcessInfo.processInfo.environment
        // GUI apps start with a minimal PATH; ffmpeg and ffprobe live here.
        environment["PATH"] = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        environment["RECO_MINIMAP_LAUNCHER"] = "1"
        environment["RECO_MINIMAP_NO_UPDATE_CHECK"] = "1"
        if let ort = payload.onnxRuntime { environment["ORT_DYLIB_PATH"] = ort.path }
        process.environment = environment
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = log
        process.standardError = log
        log.write(Data("\n--- Reco Minimap launcher: starting payload \(payload.version) on port \(port) ---\n".utf8))
        try process.run()
        self.process = process

        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if !process.isRunning {
                self.process = nil
                throw WorkerError.exited(process.terminationStatus)
            }
            if await Self.fetch(port: port, path: "/api/init") != nil { return }
            try await Task.sleep(nanoseconds: 250_000_000)
        }
        stop()
        throw WorkerError.timeout
    }

    /// Stops the program and its whole process group - its jobs and their ffmpeg processes too
    /// (SIGTERM, then SIGKILL after a few seconds). Process starts the child as a group leader.
    func stop() {
        guard let process else { return }
        self.process = nil
        guard process.isRunning else { return }
        let pid = process.processIdentifier
        kill(-pid, SIGTERM)
        kill(pid, SIGTERM)
        let deadline = Date().addingTimeInterval(4)
        while process.isRunning && Date() < deadline { Thread.sleep(forTimeInterval: 0.1) }
        if process.isRunning { kill(-pid, SIGKILL); kill(pid, SIGKILL) }
    }

    /// True while a job runs (processing, calibration, live mode ...).
    func isBusy() async -> Bool {
        guard let object = await Self.fetch(port: port, path: "/api/status") else { return false }
        return (object["busy"] as? Bool) == true
    }

    // MARK: Helpers

    private static func fetch(port: Int, path: String) async -> [String: Any]? {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:\(port)\(path)")!)
        request.timeoutInterval = 2
        guard
            let (data, response) = try? await URLSession.shared.data(for: request),
            (response as? HTTPURLResponse)?.statusCode == 200
        else { return nil }
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
    }

    static func firstFreePort(startingAt start: Int) -> Int {
        for port in start..<(start + 40) where isPortFree(UInt16(port)) { return port }
        return start
    }

    private static func isPortFree(_ port: UInt16) -> Bool {
        let descriptor = socket(AF_INET, SOCK_STREAM, 0)
        guard descriptor >= 0 else { return false }
        defer { close(descriptor) }
        // Connections of the previous run in TIME_WAIT must not count as "busy", or the port - and
        // with it the web view's per-origin settings (language, hints) - would change on a quick restart.
        var reuse: Int32 = 1
        setsockopt(descriptor, SOL_SOCKET, SO_REUSEADDR, &reuse, socklen_t(MemoryLayout<Int32>.size))
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian
        address.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))
        return withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) == 0
            }
        }
    }
}
