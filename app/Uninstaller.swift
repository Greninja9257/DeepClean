import AppKit
import SwiftUI

@MainActor
final class UninstallerModel: ObservableObject {
    enum Sort: String, CaseIterable { case size = "Size", name = "Name" }

    @Published var apps: [AppInfo] = []
    @Published var loading = false
    @Published var search = ""
    @Published var sort: Sort = .size
    @Published var current: AppInfo?
    @Published var related: [ScanItem] = []
    @Published var chosen: Set<String> = []
    @Published var loadingRelated = false
    @Published var working = false
    @Published var message: String?

    var visibleApps: [AppInfo] {
        let q = search.trimmingCharacters(in: .whitespaces).lowercased()
        let list = q.isEmpty ? apps : apps.filter { $0.name.lowercased().contains(q) || $0.bundleId.lowercased().contains(q) }
        return sort == .size ? list.sorted { $0.size > $1.size } : list.sorted { $0.name.lowercased() < $1.name.lowercased() }
    }

    var chosenItems: [ScanItem] { related.filter { chosen.contains($0.id) } }
    var chosenBytes: UInt64 { chosenItems.reduce(0) { $0 + $1.bytes } }

    func load() {
        guard !loading else { return }
        loading = true
        Task {
            if case .apps(let a)? = await Engine.fetch(["apps-json"]) { apps = a }
            loading = false
        }
    }

    func select(_ app: AppInfo) {
        current = app
        related = []
        message = nil
        loadingRelated = true
        Task {
            if case .scanResult(let r)? = await Engine.fetch(["uninstall-scan-json", app.path]), current == app {
                related = r.items
                chosen = Set(r.items.filter(\.defaultOn).map(\.id))
            }
            loadingRelated = false
        }
    }

    func toggle(_ item: ScanItem) {
        if chosen.contains(item.id) { chosen.remove(item.id) } else { chosen.insert(item.id) }
    }

    func uninstall() {
        guard let app = current, !chosenItems.isEmpty else { return }
        working = true
        Task {
            await quit(bundleId: app.bundleId)
            let summary = await Engine.execute(chosenItems, home: NSHomeDirectory())
            working = false
            if summary.problems.isEmpty {
                message = "\(app.name) was moved to the Trash (\(formatBytes(summary.trashed + summary.freed)))."
                apps.removeAll { $0 == app }
                current = nil
                related = []
            } else {
                message = summary.problems.joined(separator: "\n")
                select(app)
                load()
            }
            History.shared.reload()
        }
    }

    /// Ask a running copy of the app to quit, waiting up to five seconds.
    private func quit(bundleId: String) async {
        let running = NSRunningApplication.runningApplications(withBundleIdentifier: bundleId)
        guard !running.isEmpty else { return }
        running.forEach { $0.terminate() }
        for _ in 0..<50 where running.contains(where: { !$0.isTerminated }) {
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
    }
}

struct AppIcon: View {
    let path: String
    var size: CGFloat = 32
    var body: some View {
        Image(nsImage: NSWorkspace.shared.icon(forFile: path))
            .resizable()
            .interpolation(.high)
            .frame(width: size, height: size)
    }
}

struct UninstallerView: View {
    @ObservedObject var model: UninstallerModel

    var body: some View {
        HStack(spacing: 0) {
            appList
                .frame(width: 300)
            Divider()
            detail
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .onAppear { if model.apps.isEmpty { model.load() } }
    }

    private var appList: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Search apps", text: $model.search).textFieldStyle(.plain)
                SoftMenu(title: model.sort.rawValue, symbol: "arrow.up.arrow.down") {
                    ForEach(UninstallerModel.Sort.allCases, id: \.self) { s in
                        Button(s.rawValue) { model.sort = s }
                    }
                }
            }
            .padding(10)
            .background(RoundedRectangle(cornerRadius: 9).fill(Color.primary.opacity(0.05)))
            .padding(.horizontal, 14).padding(.top, 22).padding(.bottom, 10)

            if model.loading && model.apps.isEmpty {
                Spacer()
                ProgressView("Measuring apps…").controlSize(.small)
                Spacer()
            } else {
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(model.visibleApps) { app in
                            AppRow(app: app, selected: model.current == app)
                                .onTapGesture { model.select(app) }
                        }
                    }
                    .padding(.horizontal, 8).padding(.bottom, 10)
                }
            }
        }
    }

    @ViewBuilder private var detail: some View {
        if let app = model.current {
            VStack(spacing: 0) {
                HStack(spacing: 16) {
                    AppIcon(path: app.path, size: 64)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(app.name).font(.system(size: 22, weight: .bold))
                        Text([app.version, app.bundleId].filter { !$0.isEmpty }.joined(separator: " · "))
                            .font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer()
                    VStack(alignment: .trailing, spacing: 2) {
                        Text(formatBytes(model.chosenBytes))
                            .font(.system(size: 22, weight: .bold, design: .rounded)).monospacedDigit()
                        Text("selected").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 24).padding(.top, 22).padding(.bottom, 16)
                Divider().opacity(0.6)

                if model.loadingRelated {
                    Spacer(); ProgressView().controlSize(.small); Spacer()
                } else {
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            ForEach(model.related) { item in
                                RelatedRow(item: item, on: model.chosen.contains(item.id))
                                    .onTapGesture { model.toggle(item) }
                            }
                        }
                        .padding(.vertical, 8)
                    }
                }
                footer
            }
        } else {
            VStack(spacing: 14) {
                Image(systemName: "square.dashed.inset.filled")
                    .font(.system(size: 44, weight: .light)).foregroundStyle(.tertiary)
                Text("Pick an app to remove it and everything it left behind")
                    .foregroundStyle(.secondary)
                if let m = model.message {
                    Label(m, systemImage: "checkmark.circle.fill").foregroundStyle(.green).font(.system(size: 13))
                }
            }
        }
    }

    private var footer: some View {
        HStack(spacing: 14) {
            if let m = model.message {
                Text(m).font(.system(size: 12)).foregroundStyle(.orange).lineLimit(2)
            } else {
                Text("Everything goes to the Trash, so you can put it back.")
                    .font(.system(size: 12)).foregroundStyle(.secondary)
            }
            Spacer()
            if model.chosenItems.contains(where: \.admin) {
                Label("Asks for your password", systemImage: "lock.fill").font(.system(size: 12)).foregroundStyle(.secondary)
            }
            PrimaryButton(title: model.working ? "Uninstalling…" : "Uninstall", symbol: "trash",
                          disabled: model.chosenItems.isEmpty || model.working) { model.uninstall() }
        }
        .padding(.horizontal, 24).padding(.vertical, 14)
        .background(.bar)
        .overlay(Divider(), alignment: .top)
    }
}

private struct AppRow: View {
    let app: AppInfo
    let selected: Bool
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 10) {
            AppIcon(path: app.path, size: 30)
            VStack(alignment: .leading, spacing: 1) {
                Text(app.name).font(.system(size: 13, weight: .medium)).lineLimit(1)
                Text(app.version).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            Text(formatBytes(app.size)).font(.system(size: 12, design: .rounded)).monospacedDigit().foregroundStyle(.secondary)
        }
        .padding(.horizontal, 10).padding(.vertical, 6)
        .background(RoundedRectangle(cornerRadius: 8)
            .fill(selected ? Color.accentColor.opacity(0.18) : hovering ? Color.primary.opacity(0.05) : .clear))
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
    }
}

private struct RelatedRow: View {
    let item: ScanItem
    let on: Bool
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 12) {
            CheckBox(state: on ? .all : .none, tint: .red)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 5) {
                    Text(item.name).font(.system(size: 13)).lineLimit(1).truncationMode(.middle)
                    if item.admin { Image(systemName: "lock.fill").font(.system(size: 9)).foregroundStyle(.secondary) }
                }
                Text(item.category).font(.system(size: 11)).foregroundStyle(.secondary)
            }
            Spacer(minLength: 12)
            Text(item.size.map(formatBytes) ?? "—")
                .font(.system(size: 12, weight: .medium, design: .rounded)).monospacedDigit()
        }
        .padding(.horizontal, 24).padding(.vertical, 6)
        .background(hovering ? Color.primary.opacity(0.035) : .clear)
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        .contextMenu {
            if let p = item.paths?.first {
                Button("Reveal in Finder") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: p)]) }
            }
        }
        .help(item.paths?.first ?? "")
    }
}
