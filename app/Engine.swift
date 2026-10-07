import Foundation

// MARK: - Wire types (mirror src/api.rs)

struct ScanItem: Codable, Identifiable, Hashable {
    let id: String
    let group: String
    let category: String
    let name: String
    let kind: String          // "delete" | "command"
    let paths: [String]?
    let command: String?
    let size: UInt64?
    let defaultOn: Bool
    let note: String
    let admin: Bool
    let trash: Bool?

    var bytes: UInt64 { size ?? 0 }
    var movesToTrash: Bool { trash ?? false }
}

struct AppInfo: Codable, Identifiable, Hashable {
    let name: String
    let path: String
    let bundleId: String
    let version: String
    let size: UInt64
    let admin: Bool
    var id: String { path }
}

struct AnalysisEntry: Codable, Identifiable, Hashable {
    let name: String
    let path: String
    let size: UInt64
    let isDir: Bool
    let items: UInt64
    var id: String { path }
}

struct Analysis: Codable {
    let path: String
    let total: UInt64
    let entries: [AnalysisEntry]
}

private struct AppsEnvelope: Decodable { let items: [AppInfo] }

/// Combined result of a clean (regular + password-protected parts).
struct CleanSummary {
    var outcomes: [CleanOutcome] = []
    var refused: [String] = []
    var problems: [String] = []
    var freed: UInt64 { outcomes.reduce(0) { $0 + $1.freed } }
    var trashed: UInt64 { outcomes.reduce(0) { $0 + ($1.trashed ?? 0) } }
}

struct DiskInfo: Codable { let total: UInt64?; let free: UInt64? }

struct ScanResult: Codable {
    let fda: Bool
    let home: String
    let disk: DiskInfo
    let elapsedMs: UInt64
    let items: [ScanItem]
}

struct CleanOutcome: Codable, Hashable {
    let id: String
    let name: String
    let freed: UInt64
    let trashed: UInt64?
    let failures: UInt64
    let error: String?
}

struct CleanReport: Codable {
    let outcomes: [CleanOutcome]
    let refused: [String]
    let freed: UInt64
    let freeBefore: UInt64?
    let freeAfter: UInt64?
}

private struct Envelope: Decodable { let type: String }
private struct ScanProgress: Decodable { let files: UInt64; let bytes: UInt64 }
private struct PhaseEvent: Decodable { let name: String }
private struct CleanProgress: Decodable { let bytesDone: UInt64; let bytesTotal: UInt64; let files: UInt64 }
private struct EngineError: Decodable { let message: String }

private struct RequestItem: Encodable {
    let id: String
    let name: String
    let size: UInt64?
    let paths: [String]?
    let command: String?
    let trash: Bool
}

enum EngineEvent {
    case scanProgress(files: UInt64, bytes: UInt64)
    case phase(String)
    case scanResult(ScanResult)
    case cleanProgress(done: UInt64, total: UInt64, files: UInt64)
    case cleanDone(CleanReport)
    case apps([AppInfo])
    case analysis(Analysis)
    case failure(String)
}

// MARK: - Engine process

/// Runs the bundled Rust engine and streams its line-delimited JSON events.
enum Engine {
    static var url: URL {
        Bundle.main.url(forAuxiliaryExecutable: "deepclean-engine")
            ?? URL(fileURLWithPath: "/usr/local/bin/deepclean")
    }

    private static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    static func decode(_ line: Data) -> EngineEvent? {
        guard let env = try? decoder.decode(Envelope.self, from: line) else { return nil }
        switch env.type {
        case "progress":
            if let p = try? decoder.decode(ScanProgress.self, from: line) {
                return .scanProgress(files: p.files, bytes: p.bytes)
            }
            if let p = try? decoder.decode(CleanProgress.self, from: line) {
                return .cleanProgress(done: p.bytesDone, total: p.bytesTotal, files: p.files)
            }
        case "phase":
            return (try? decoder.decode(PhaseEvent.self, from: line)).map { .phase($0.name) }
        case "result":
            do { return .scanResult(try decoder.decode(ScanResult.self, from: line)) }
            catch { return .failure("Couldn't read scan results: \(error.localizedDescription)") }
        case "apps":
            return (try? decoder.decode(AppsEnvelope.self, from: line)).map { .apps($0.items) }
        case "analysis":
            return (try? decoder.decode(Analysis.self, from: line)).map { .analysis($0) }
        case "done":
            return (try? decoder.decode(CleanReport.self, from: line)).map { .cleanDone($0) }
        case "error":
            return (try? decoder.decode(EngineError.self, from: line)).map { .failure($0.message) }
        default: break
        }
        return nil
    }

    /// Launch the engine and deliver each event on the main actor.
    static func run(_ args: [String], stdin: Data? = nil,
                    onEvent: @escaping @MainActor (EngineEvent) -> Void) async {
        // Keep App Nap from throttling event delivery while the window is in
        // the background (every tool streams engine output through here).
        let activity = ProcessInfo.processInfo.beginActivity(options: .userInitiated, reason: "DeepClean engine")
        defer { ProcessInfo.processInfo.endActivity(activity) }
        let process = Process()
        process.executableURL = url
        process.arguments = args
        let out = Pipe()
        process.standardOutput = out
        process.standardError = FileHandle.nullDevice
        let input = Pipe()
        process.standardInput = input
        do { try process.run() } catch {
            await onEvent(.failure("Couldn't start the cleaning engine: \(error.localizedDescription)"))
            return
        }
        if let stdin {
            input.fileHandleForWriting.write(stdin)
        }
        try? input.fileHandleForWriting.close()
        do {
            for try await line in out.fileHandleForReading.bytes.lines {
                if let ev = decode(Data(line.utf8)) { await onEvent(ev) }
            }
        } catch {
            await onEvent(.failure(error.localizedDescription))
        }
        process.waitUntilExit()
    }

    static func request(for items: [ScanItem]) -> Data {
        let req = ["items": items.map {
            RequestItem(id: $0.id, name: $0.name, size: $0.size, paths: $0.paths, command: $0.command, trash: $0.movesToTrash)
        }]
        return (try? JSONEncoder().encode(req)) ?? Data()
    }

    /// Clean admin-only items: macOS asks for the user's password once.
    static func runPrivileged(_ items: [ScanItem], home: String) async -> EngineEvent {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("deepclean-\(UUID().uuidString).json")
        do { try request(for: items).write(to: file) } catch {
            return .failure("Couldn't prepare the request: \(error.localizedDescription)")
        }
        defer { try? FileManager.default.removeItem(at: file) }

        func q(_ s: String) -> String {
            "quoted form of \"" + s.replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "\"", with: "\\\"") + "\""
        }
        let script = "do shell script (\(q(url.path))) & \" clean-json --input \" & (\(q(file.path))) & \" --home \" & (\(q(home))) with administrator privileges without altering line endings"

        return await withCheckedContinuation { cont in
            DispatchQueue.global().async {
                let p = Process()
                p.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
                p.arguments = ["-e", script]
                let out = Pipe(), err = Pipe()
                p.standardOutput = out
                p.standardError = err
                do { try p.run() } catch {
                    cont.resume(returning: .failure(error.localizedDescription)); return
                }
                let data = out.fileHandleForReading.readDataToEndOfFile()
                let errText = String(data: err.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
                p.waitUntilExit()
                let lines = data.split(separator: UInt8(ascii: "\n"))
                for line in lines.reversed() {
                    if case .cleanDone(let r)? = decode(Data(line)) {
                        cont.resume(returning: .cleanDone(r)); return
                    }
                }
                let cancelled = errText.contains("-128")
                cont.resume(returning: .failure(cancelled ? "cancelled" : "Administrator clean failed. \(errText)"))
            }
        }
    }

    /// Clean `items`: the regular ones directly, then the password-protected
    /// ones through a single administrator prompt.
    @MainActor
    static func execute(_ items: [ScanItem], home: String,
                        onProgress: @escaping @MainActor (_ done: UInt64, _ files: UInt64) -> Void = { _, _ in },
                        onPassword: @escaping @MainActor () -> Void = {}) async -> CleanSummary {
        var summary = CleanSummary()
        let regular = items.filter { !$0.admin }
        let privileged = items.filter(\.admin)
        if !regular.isEmpty {
            await run(["clean-json"], stdin: request(for: regular)) { ev in
                switch ev {
                case .cleanProgress(let done, _, let files): onProgress(done, files)
                case .cleanDone(let r):
                    summary.outcomes += r.outcomes
                    summary.refused += r.refused
                case .failure(let m): summary.problems.append(m)
                default: break
                }
            }
        }
        if !privileged.isEmpty {
            onPassword()
            switch await runPrivileged(privileged, home: home) {
            case .cleanDone(let r):
                summary.outcomes += r.outcomes
                summary.refused += r.refused
            case .failure(let m) where m == "cancelled":
                summary.problems.append("Items needing your password were skipped because the prompt was cancelled.")
            case .failure(let m):
                summary.problems.append(m)
            default: break
            }
        }
        for o in summary.outcomes {
            if let e = o.error { summary.problems.append("\(o.name): \(e)") }
            else if o.failures > 0 { summary.problems.append("\(o.name): \(o.failures) items were in use or protected") }
        }
        if !summary.refused.isEmpty { summary.problems.append("\(summary.refused.count) paths were skipped by safety rules") }
        return summary
    }

    /// Run a one-shot engine command and return its final event of interest.
    @MainActor
    static func fetch(_ args: [String], onProgress: @escaping @MainActor (UInt64) -> Void = { _ in }) async -> EngineEvent? {
        var result: EngineEvent?
        await run(args) { ev in
            switch ev {
            case .scanProgress(let files, _): onProgress(files)
            case .apps, .analysis, .scanResult, .failure: result = ev
            default: break
            }
        }
        return result
    }

    /// Add a path to the never-clean list.
    static func whitelist(_ path: String) {
        let p = Process()
        p.executableURL = url
        p.arguments = ["whitelist", "add", path]
        p.standardOutput = FileHandle.nullDevice
        try? p.run()
        p.waitUntilExit()
    }
}
