import AppKit
import SwiftUI

@MainActor
final class SpaceLensModel: ObservableObject {
    @Published var path = NSHomeDirectory()
    @Published var analysis: Analysis?
    @Published var loading = false
    @Published var filesSeen: UInt64 = 0
    @Published var chosen: Set<String> = []
    @Published var message: String?
    private var backStack: [String] = []
    /// Results already computed this session, so going back is instant.
    private var cache: [String: Analysis] = [:]

    var canGoBack: Bool { !backStack.isEmpty }
    var chosenEntries: [AnalysisEntry] { analysis?.entries.filter { chosen.contains($0.id) } ?? [] }

    func open(_ newPath: String, remember: Bool = true) {
        if remember, newPath != path { backStack.append(path) }
        path = newPath
        chosen = []
        message = nil
        if let hit = cache[newPath] {
            analysis = hit
            return
        }
        analysis = nil
        loading = true
        filesSeen = 0
        Task {
            let ev = await Engine.fetch(["analyze-json", newPath]) { [weak self] files in self?.filesSeen = files }
            if case .analysis(let a)? = ev {
                cache[a.path] = a
                if path == newPath { analysis = a }
            }
            if path == newPath { loading = false }
        }
    }

    func back() {
        guard let prev = backStack.popLast() else { return }
        open(prev, remember: false)
    }

    func refresh() {
        cache[path] = nil
        open(path, remember: false)
    }

    func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.directoryURL = URL(fileURLWithPath: path)
        panel.prompt = "Analyze"
        if panel.runModal() == .OK, let url = panel.url { open(url.path) }
    }

    func toggle(_ e: AnalysisEntry) {
        if chosen.contains(e.id) { chosen.remove(e.id) } else { chosen.insert(e.id) }
    }

    func trashChosen() {
        let entries = chosenEntries
        guard !entries.isEmpty else { return }
        let items = entries.map {
            ScanItem(id: $0.path, group: "analyze", category: "", name: $0.name, kind: "delete",
                     paths: [$0.path], command: nil, size: $0.size, defaultOn: true, note: "", admin: false, trash: true)
        }
        Task {
            let s = await Engine.execute(items, home: NSHomeDirectory())
            message = s.problems.isEmpty
                ? "Moved \(formatBytes(s.trashed)) to the Trash."
                : s.problems.joined(separator: "\n")
            // everything above this folder changed size too
            cache = [:]
            refresh()
            History.shared.reload()
        }
    }
}

struct SpaceLensView: View {
    @ObservedObject var model: SpaceLensModel

    var body: some View {
        VStack(spacing: 0) {
            toolbar
                .padding(.horizontal, 24).padding(.top, 22).padding(.bottom, 12)
            Divider().opacity(0.6)
            content
            footer
        }
        .onAppear { if model.analysis == nil && !model.loading { model.open(model.path, remember: false) } }
    }

    private var crumbs: [(name: String, path: String)] {
        let home = NSHomeDirectory()
        var parts: [(String, String)] = []
        var url = URL(fileURLWithPath: model.path)
        while true {
            let p = url.path
            parts.insert((p == home ? "Home" : (p == "/" ? "Macintosh HD" : url.lastPathComponent), p), at: 0)
            if p == home || p == "/" { break }
            url.deleteLastPathComponent()
        }
        return parts
    }

    private var toolbar: some View {
        HStack(spacing: 10) {
            RoundIconButton(symbol: "chevron.left", disabled: !model.canGoBack) { model.back() }
            HStack(spacing: 4) {
                ForEach(Array(crumbs.enumerated()), id: \.offset) { i, c in
                    if i > 0 { Image(systemName: "chevron.right").font(.system(size: 9, weight: .bold)).foregroundStyle(.tertiary) }
                    Button(c.name) { model.open(c.path) }
                        .buttonStyle(.plain)
                        .font(.system(size: 13, weight: i == crumbs.count - 1 ? .semibold : .regular))
                        .foregroundStyle(i == crumbs.count - 1 ? .primary : .secondary)
                }
            }
            .lineLimit(1)
            Spacer()
            if let a = model.analysis {
                Text(formatBytes(a.total)).font(.system(size: 15, weight: .bold, design: .rounded)).monospacedDigit()
            }
            RoundIconButton(symbol: "arrow.clockwise") { model.refresh() }
            SoftButton(title: "Choose Folder", symbol: "folder") { model.chooseFolder() }
        }
    }

    @ViewBuilder private var content: some View {
        if model.loading {
            VStack(spacing: 10) {
                ProgressView()
                Text("\(model.filesSeen.formatted()) files measured")
                    .font(.system(size: 12)).foregroundStyle(.secondary).monospacedDigit()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let a = model.analysis, !a.entries.isEmpty {
            let biggest = max(a.entries.first?.size ?? 1, 1)
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(a.entries) { e in
                        LensRow(entry: e, fraction: Double(e.size) / Double(biggest),
                                share: a.total > 0 ? Double(e.size) / Double(a.total) : 0,
                                on: model.chosen.contains(e.id),
                                toggle: { model.toggle(e) },
                                open: { if e.isDir { model.open(e.path) } })
                    }
                }
                .padding(.vertical, 6)
            }
        } else {
            ContentUnavailableView("Nothing here", systemImage: "folder",
                                   description: Text("This folder is empty or DeepClean can't read it."))
        }
    }

    private var footer: some View {
        HStack(spacing: 14) {
            Text(model.message ?? "Click a folder to look inside. Tick items to move them to the Trash.")
                .font(.system(size: 12)).foregroundStyle(model.message == nil ? Color.secondary : Color.orange)
                .lineLimit(2)
            Spacer()
            let bytes = model.chosenEntries.reduce(0) { $0 + $1.size }
            PrimaryButton(title: model.chosen.isEmpty ? "Move to Trash" : "Move \(formatBytes(bytes)) to Trash",
                          symbol: "trash", disabled: model.chosen.isEmpty) { model.trashChosen() }
        }
        .padding(.horizontal, 24).padding(.vertical, 14)
        .background(.bar)
        .overlay(Divider(), alignment: .top)
    }
}

private struct LensRow: View {
    let entry: AnalysisEntry
    let fraction: Double
    let share: Double
    let on: Bool
    let toggle: () -> Void
    let open: () -> Void
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 12) {
            Button(action: toggle) { CheckBox(state: on ? .all : .none, tint: .pink) }.buttonStyle(.plain)
            Image(nsImage: NSWorkspace.shared.icon(forFile: entry.path))
                .resizable().frame(width: 22, height: 22)
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(entry.name).font(.system(size: 13)).lineLimit(1).truncationMode(.middle)
                    if entry.isDir {
                        Text("\(entry.items) item\(entry.items == 1 ? "" : "s")")
                            .font(.system(size: 11)).foregroundStyle(.tertiary)
                    }
                }
                GeometryReader { g in
                    ZStack(alignment: .leading) {
                        Capsule().fill(Color.primary.opacity(0.06))
                        Capsule().fill(brandGradient).frame(width: max(3, g.size.width * fraction))
                    }
                }
                .frame(height: 5)
            }
            Text(String(format: "%.0f%%", share * 100))
                .font(.system(size: 11)).foregroundStyle(.tertiary).monospacedDigit()
                .frame(width: 36, alignment: .trailing)
            Text(formatBytes(entry.size))
                .font(.system(size: 12, weight: .medium, design: .rounded)).monospacedDigit()
                .frame(width: 76, alignment: .trailing)
            Image(systemName: "chevron.right")
                .font(.system(size: 10, weight: .bold))
                .foregroundStyle(entry.isDir ? .tertiary : .quaternary)
                .opacity(entry.isDir ? 1 : 0)
        }
        .padding(.horizontal, 24).padding(.vertical, 7)
        .background(hovering ? Color.primary.opacity(0.035) : .clear)
        .contentShape(Rectangle())
        .onTapGesture(perform: open)
        .onHover { hovering = $0 }
        .contextMenu {
            Button("Reveal in Finder") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: entry.path)]) }
            Button(on ? "Untick" : "Tick for Trash", action: toggle)
        }
    }
}
