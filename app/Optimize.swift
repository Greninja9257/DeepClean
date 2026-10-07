import SwiftUI

@MainActor
final class OptimizeModel: ObservableObject {
    enum Status: Equatable { case idle, running, ok, failed(String) }

    @Published var tasks: [ScanItem] = []
    @Published var chosen: Set<String> = []
    @Published var status: [String: Status] = [:]
    @Published var running = false

    func load() {
        Task {
            if case .scanResult(let r)? = await Engine.fetch(["optimize-json"]) {
                tasks = r.items
                chosen = Set(r.items.filter(\.defaultOn).map(\.id))
            }
        }
    }

    func toggle(_ t: ScanItem) {
        guard !running else { return }
        if chosen.contains(t.id) { chosen.remove(t.id) } else { chosen.insert(t.id) }
    }

    func run() {
        let picked = tasks.filter { chosen.contains($0.id) }
        guard !picked.isEmpty else { return }
        running = true
        picked.forEach { status[$0.id] = .running }
        Task {
            let s = await Engine.execute(picked, home: NSHomeDirectory())
            let byId = Dictionary(s.outcomes.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
            for t in picked {
                if let o = byId[t.id] {
                    status[t.id] = o.error.map { .failed($0) } ?? .ok
                } else {
                    status[t.id] = .failed(s.problems.first ?? "Skipped")
                }
            }
            running = false
            History.shared.reload()
        }
    }

    static func symbol(for id: String) -> String {
        switch id {
        case "cmd:flush-dns": "network"
        case "cmd:reset-quicklook": "eye"
        case "cmd:rebuild-launchservices": "arrow.triangle.2.circlepath"
        case "cmd:purge-memory": "memorychip"
        case "cmd:restart-dock": "dock.rectangle"
        case "cmd:restart-finder": "macwindow"
        case "cmd:reindex-spotlight": "magnifyingglass"
        case "cmd:verify-disk": "internaldrive"
        default: "wand.and.stars"
        }
    }
}

struct OptimizeView: View {
    @ObservedObject var model: OptimizeModel

    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Optimize").font(.system(size: 30, weight: .bold, design: .rounded))
                Text("Quick maintenance that fixes common slowdowns and glitches.")
                    .font(.system(size: 13)).foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 28).padding(.top, 22).padding(.bottom, 18)
            Divider().opacity(0.6)

            ScrollView {
                LazyVStack(spacing: 10) {
                    ForEach(model.tasks) { t in
                        TaskCard(task: t, on: model.chosen.contains(t.id), status: model.status[t.id] ?? .idle)
                            .onTapGesture { model.toggle(t) }
                    }
                }
                .padding(24)
            }

            HStack {
                if model.tasks.contains(where: { model.chosen.contains($0.id) && $0.admin }) {
                    Label("Asks for your password", systemImage: "lock.fill")
                        .font(.system(size: 12)).foregroundStyle(.secondary)
                }
                Spacer()
                PrimaryButton(title: model.running ? "Running…" : "Run \(model.chosen.count) Task\(model.chosen.count == 1 ? "" : "s")",
                              symbol: "wand.and.stars",
                              disabled: model.chosen.isEmpty || model.running) { model.run() }
            }
            .padding(.horizontal, 28).padding(.vertical, 16)
            .background(.bar)
            .overlay(Divider(), alignment: .top)
        }
        .onAppear { if model.tasks.isEmpty { model.load() } }
    }
}

private struct TaskCard: View {
    let task: ScanItem
    let on: Bool
    let status: OptimizeModel.Status

    var body: some View {
        HStack(spacing: 14) {
            CheckBox(state: on ? .all : .none, tint: .orange)
            ZStack {
                Circle().fill(Color.orange.opacity(0.13))
                Image(systemName: OptimizeModel.symbol(for: task.id))
                    .font(.system(size: 15, weight: .semibold)).foregroundStyle(.orange)
            }
            .frame(width: 36, height: 36)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 5) {
                    Text(task.name).font(.system(size: 14, weight: .semibold))
                    if task.admin { Image(systemName: "lock.fill").font(.system(size: 9)).foregroundStyle(.secondary) }
                }
                Text(task.note).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(2)
            }
            Spacer()
            switch status {
            case .idle: EmptyView()
            case .running: ProgressView().controlSize(.small)
            case .ok: Image(systemName: "checkmark.circle.fill").foregroundStyle(.green).font(.system(size: 18))
            case .failed(let m):
                Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.orange).font(.system(size: 18)).help(m)
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Color.primary.opacity(0.06)))
        .contentShape(Rectangle())
    }
}
