import AppKit
import SwiftUI

struct GroupInfo {
    let id: String
    let title: String
    let subtitle: String
    let symbol: String
    let tint: Color
}

let groupOrder: [GroupInfo] = [
    .init(id: "junk", title: "System Junk", subtitle: "Caches, logs & Trash", symbol: "trash", tint: .blue),
    .init(id: "dev", title: "Developer Tools", subtitle: "Package caches, Xcode, Gradle, Cargo…", symbol: "hammer", tint: .orange),
    .init(id: "projects", title: "Project Builds", subtitle: "node_modules, target, .venv, build…", symbol: "shippingbox", tint: .purple),
    .init(id: "apps", title: "App Caches", subtitle: "Electron apps, editors, games", symbol: "square.stack.3d.up", tint: .indigo),
    .init(id: "browsers", title: "Browsers", subtitle: "Chrome, Safari, Arc, Firefox…", symbol: "globe", tint: .teal),
    .init(id: "downloads", title: "Downloads", subtitle: "Installers, extracted archives, old files", symbol: "arrow.down.circle", tint: .green),
    .init(id: "large", title: "Large Files", subtitle: "Big files and duplicates to review", symbol: "doc.on.doc", tint: .pink),
    .init(id: "leftovers", title: "App Leftovers", subtitle: "Data from apps you've uninstalled", symbol: "puzzlepiece.extension", tint: .brown),
    .init(id: "system", title: "System", subtitle: "System caches and snapshots. Needs your password", symbol: "lock.shield", tint: .gray),
]

enum Phase: Equatable {
    case home, scanning, results, cleaning, done
}

func formatBytes(_ b: UInt64) -> String {
    b == 0 ? "0 KB" : ByteCountFormatter.string(fromByteCount: Int64(b), countStyle: .file)
}

struct DiskSpace {
    var total: UInt64 = 0
    var free: UInt64 = 0
    var used: UInt64 { total > free ? total - free : 0 }

    static func current() -> DiskSpace {
        let url = URL(fileURLWithPath: NSHomeDirectory())
        let v = try? url.resourceValues(forKeys: [.volumeTotalCapacityKey, .volumeAvailableCapacityForImportantUsageKey])
        return DiskSpace(total: UInt64(v?.volumeTotalCapacity ?? 0),
                         free: UInt64(v?.volumeAvailableCapacityForImportantUsage ?? 0))
    }
}

@MainActor
final class AppModel: ObservableObject {
    @Published var section: SidebarSection? = .clean
    @Published var phase: Phase = .home
    @Published var search = ""
    @Published var disk = DiskSpace.current()
    @Published var hasFullDiskAccess = AppModel.checkFullDiskAccess()

    // scanning
    @Published var scannedFiles: UInt64 = 0
    @Published var scannedBytes: UInt64 = 0
    @Published var phasesDone: Set<String> = []

    // results
    @Published var items: [ScanItem] = []
    @Published var selected: Set<String> = []
    @Published var expanded: Set<String> = []
    @Published var scanSeconds: Double = 0
    var home = NSHomeDirectory()

    // cleaning
    @Published var cleanDone: UInt64 = 0
    @Published var cleanTotal: UInt64 = 1
    @Published var filesRemoved: UInt64 = 0
    @Published var cleaningLabel = ""

    // done
    @Published var freed: UInt64 = 0
    @Published var trashed: UInt64 = 0
    @Published var freeBefore: UInt64 = 0
    @Published var problems: [String] = []
    @Published var errorMessage: String?

    /// Keeps macOS App Nap from throttling us while the window is in the
    /// background; otherwise engine events stall until the app is clicked.
    private var activity: NSObjectProtocol?

    private func beginWork(_ reason: String) {
        endWork()
        activity = ProcessInfo.processInfo.beginActivity(options: [.userInitiated, .idleSystemSleepDisabled], reason: reason)
    }

    private func endWork() {
        if let a = activity { ProcessInfo.processInfo.endActivity(a) }
        activity = nil
    }

    static func checkFullDiskAccess() -> Bool {
        FileHandle(forReadingAtPath: "/Library/Application Support/com.apple.TCC/TCC.db") != nil
    }

    // MARK: derived

    /// Items in a group that match the filter text.
    func items(in group: String) -> [ScanItem] {
        let q = search.trimmingCharacters(in: .whitespaces).lowercased()
        return items.filter {
            $0.group == group && (q.isEmpty || $0.name.lowercased().contains(q)
                || $0.category.lowercased().contains(q) || $0.note.lowercased().contains(q))
        }
    }

    func selectRecommended() { selected = Set(items.filter(\.defaultOn).map(\.id)) }
    func selectAll() { selected = Set(items.map(\.id)) }
    func selectNone() { selected = [] }

    var visibleGroups: [GroupInfo] {
        groupOrder.filter { g in !items(in: g.id).isEmpty }
    }

    var selectedItems: [ScanItem] { items.filter { selected.contains($0.id) } }
    var selectedBytes: UInt64 { selectedItems.reduce(0) { $0 + $1.bytes } }
    var foundBytes: UInt64 { items.reduce(0) { $0 + $1.bytes } }
    var selectedAdminCount: Int { selectedItems.filter(\.admin).count }

    func bytes(in group: String, selectedOnly: Bool = false) -> UInt64 {
        items(in: group).filter { !selectedOnly || selected.contains($0.id) }.reduce(0) { $0 + $1.bytes }
    }

    enum Tri { case none, some, all }

    func state(of group: String) -> Tri {
        let ids = items(in: group).map(\.id)
        let n = ids.filter(selected.contains).count
        return n == 0 ? .none : (n == ids.count ? .all : .some)
    }

    /// Unchecked → the recommended items (never silently the risky ones);
    /// anything checked → clear.
    func toggleGroup(_ group: String) {
        let its = items(in: group)
        if state(of: group) != .none {
            its.forEach { selected.remove($0.id) }
        } else {
            let rec = its.filter(\.defaultOn)
            (rec.isEmpty ? its : rec).forEach { selected.insert($0.id) }
        }
    }

    func toggle(_ item: ScanItem) {
        if selected.contains(item.id) { selected.remove(item.id) } else { selected.insert(item.id) }
    }

    // MARK: actions

    func scan() {
        phase = .scanning
        scannedFiles = 0
        scannedBytes = 0
        phasesDone = []
        errorMessage = nil
        beginWork("Scanning for files to clean")
        Task {
            defer { self.endWork() }
            // Screenshot/demo mode: show a saved sample scan instead of running the engine.
            if let demo = ProcessInfo.processInfo.environment["DEEPCLEAN_DEMO_FILE"],
               let data = FileManager.default.contents(atPath: demo),
               case .scanResult(let r)? = Engine.decode(data) {
                self.showResult(r)
                return
            }
            await Engine.run(["scan-json"]) { [weak self] ev in
                guard let self else { return }
                switch ev {
                case .scanProgress(let f, let b):
                    self.scannedFiles = f
                    self.scannedBytes = b
                case .phase(let p):
                    withAnimation(.snappy) { _ = self.phasesDone.insert(p) }
                case .scanResult(let r):
                    self.showResult(r)
                case .failure(let m):
                    self.errorMessage = m
                    self.phase = .home
                default: break
                }
            }
            if self.phase == .scanning {
                self.errorMessage = self.errorMessage ?? "The scan ended unexpectedly."
                self.phase = .home
            }
        }
    }

    private func showResult(_ r: ScanResult) {
        home = r.home
        hasFullDiskAccess = r.fda
        scanSeconds = Double(r.elapsedMs) / 1000
        items = r.items.sorted { ($0.size ?? 0) > ($1.size ?? 0) }
        selected = Set(r.items.filter(\.defaultOn).map(\.id))
        expanded = []
        search = ""
        disk = DiskSpace.current()
        if let t = r.disk.total, let f = r.disk.free, ProcessInfo.processInfo.environment["DEEPCLEAN_DEMO_FILE"] != nil {
            disk = DiskSpace(total: t, free: f)
        }
        withAnimation(.smooth) { phase = .results }
    }

    func clean() {
        let chosen = selectedItems
        guard !chosen.isEmpty else { return }
        freeBefore = DiskSpace.current().free
        cleanDone = 0
        cleanTotal = max(1, chosen.reduce(0) { $0 + $1.bytes })
        filesRemoved = 0
        problems = []
        cleaningLabel = "Cleaning…"
        withAnimation(.smooth) { phase = .cleaning }
        beginWork("Cleaning files")

        Task {
            defer { self.endWork() }
            let summary = await Engine.execute(
                chosen, home: home,
                onProgress: { [weak self] done, files in
                    self?.cleanDone = done
                    self?.filesRemoved = files
                },
                onPassword: { [weak self] in self?.cleaningLabel = "Waiting for your password…" })
            problems = summary.problems
            freed = summary.freed
            trashed = summary.trashed
            cleanDone = cleanTotal
            disk = DiskSpace.current()
            History.shared.reload()
            withAnimation(.spring(response: 0.5, dampingFraction: 0.75)) { phase = .done }
        }
    }

    func neverClean(_ item: ScanItem) {
        guard let paths = item.paths else { return }
        if paths.count == 1 { Engine.whitelist(paths[0]) }
        withAnimation {
            items.removeAll { $0.id == item.id }
            selected.remove(item.id)
        }
    }

    func reveal(_ item: ScanItem) {
        guard let p = item.paths?.first else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: p)])
    }

    func openFullDiskAccessSettings() {
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles") {
            NSWorkspace.shared.open(url)
        }
    }

    func goHome() {
        disk = DiskSpace.current()
        hasFullDiskAccess = Self.checkFullDiskAccess()
        withAnimation(.smooth) { phase = .home }
    }
}
