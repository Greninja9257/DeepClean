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

    var bytes: UInt64 { size ?? 0 }
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
}

enum EngineEvent {
    case scanProgress(files: UInt64, bytes: UInt64)
    case phase(String)
    case scanResult(ScanResult)
    case cleanProgress(done: UInt64, total: UInt64, files: UInt64)
    case cleanDone(CleanReport)
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
            RequestItem(id: $0.id, name: $0.name, size: $0.size, paths: $0.paths, command: $0.command)
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
