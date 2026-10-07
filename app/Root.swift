import AppKit
import SwiftUI

enum SidebarSection: String, CaseIterable, Identifiable {
    case clean, uninstall, lens, optimize, status, history
    var id: String { rawValue }

    var title: String {
        switch self {
        case .clean: "Smart Clean"
        case .uninstall: "Uninstaller"
        case .lens: "Space Lens"
        case .optimize: "Optimize"
        case .status: "Status"
        case .history: "History"
        }
    }

    var symbol: String {
        switch self {
        case .clean: "sparkles"
        case .uninstall: "trash.square"
        case .lens: "chart.pie"
        case .optimize: "wand.and.stars"
        case .status: "gauge.with.dots.needle.50percent"
        case .history: "clock.arrow.circlepath"
        }
    }
}

struct RootView: View {
    @EnvironmentObject var model: AppModel
    // Owned here so each tool keeps its results when you switch sections.
    @StateObject private var uninstaller = UninstallerModel()
    @StateObject private var lens = SpaceLensModel()
    @StateObject private var optimize = OptimizeModel()

    var body: some View {
        NavigationSplitView {
            VStack(spacing: 0) {
                List(SidebarSection.allCases, selection: $model.section) { s in
                    Label(s.title, systemImage: s.symbol)
                        .font(.system(size: 13, weight: .medium))
                        .padding(.vertical, 3)
                        .tag(s)
                }
                .listStyle(.sidebar)
                sidebarDisk
            }
            .padding(.top, 8)
            .navigationSplitViewColumnWidth(min: 180, ideal: 200, max: 240)
        } detail: {
            ZStack {
                LinearGradient(colors: [Color.blue.opacity(0.06), Color.clear], startPoint: .top, endPoint: .center)
                    .ignoresSafeArea()
                switch model.section ?? .clean {
                case .clean: SmartCleanView()
                case .uninstall: UninstallerView(model: uninstaller)
                case .lens: SpaceLensView(model: lens)
                case .optimize: OptimizeView(model: optimize)
                case .status: StatusView()
                case .history: HistoryView()
                }
            }
            // The title-bar strip is only needed for the sidebar's traffic
            // lights; let the content side use it.
            .ignoresSafeArea(.container, edges: .top)
            .background(Color(nsColor: .windowBackgroundColor))
        }
        .frame(minWidth: 960, minHeight: 620)
    }

    private var sidebarDisk: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Macintosh HD").font(.system(size: 11, weight: .semibold)).foregroundStyle(.secondary)
            GeometryReader { g in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.primary.opacity(0.08))
                    Capsule().fill(brandGradient)
                        .frame(width: g.size.width * Double(model.disk.used) / Double(max(model.disk.total, 1)))
                }
            }
            .frame(height: 6)
            Text("\(formatBytes(model.disk.free)) free")
                .font(.system(size: 11)).foregroundStyle(.secondary).monospacedDigit()
        }
        .padding(14)
    }
}

/// Free space shown in the menu bar; refreshed once a minute.
@MainActor
final class MenuBarModel: ObservableObject {
    @Published var disk = DiskSpace.current()
    private var timer: Timer?

    init() {
        timer = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.disk = DiskSpace.current() }
        }
    }

    var label: String {
        ByteCountFormatter.string(fromByteCount: Int64(disk.free), countStyle: .file)
    }
}

struct MenuBarContent: View {
    @ObservedObject var bar: MenuBarModel
    @EnvironmentObject var model: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Text("\(formatBytes(bar.disk.free)) free of \(formatBytes(bar.disk.total))")
        Divider()
        Button("Scan Now") {
            show()
            model.section = .clean
            if model.phase != .scanning && model.phase != .cleaning { model.scan() }
        }
        Button("Open DeepClean") { show() }
        Divider()
        Button("Quit DeepClean") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
    }

    private func show() {
        openWindow(id: "main")
        NSApp.activate(ignoringOtherApps: true)
        bar.disk = DiskSpace.current()
    }
}
