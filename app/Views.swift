import SwiftUI

// MARK: - Shared pieces

let brandGradient = LinearGradient(
    colors: [Color(red: 0.20, green: 0.78, blue: 0.95), Color(red: 0.25, green: 0.45, blue: 1.0)],
    startPoint: .topLeading, endPoint: .bottomTrailing)

struct BrandGlyph: View {
    var size: CGFloat = 96
    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.27, style: .continuous)
            .fill(brandGradient)
            .frame(width: size, height: size)
            .overlay(
                Image(systemName: "sparkles")
                    .font(.system(size: size * 0.48, weight: .semibold))
                    .foregroundStyle(.white)
            )
            .shadow(color: .blue.opacity(0.25), radius: size * 0.18, y: size * 0.08)
    }
}

struct PrimaryButton: View {
    let title: String
    var symbol: String? = nil
    var disabled = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                if let symbol { Image(systemName: symbol) }
                Text(title)
            }
            .font(.system(size: 15, weight: .semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, 28)
            .frame(height: 44)
            .background(Capsule().fill(brandGradient).opacity(disabled ? 0.4 : 1))
            .shadow(color: .blue.opacity(disabled ? 0 : 0.3), radius: 10, y: 4)
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .keyboardShortcut(.defaultAction)
    }
}

struct DiskBar: View {
    let disk: DiskSpace
    var reclaim: UInt64 = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            GeometryReader { geo in
                let w = geo.size.width
                let total = Double(max(disk.total, 1))
                let usedW = w * Double(disk.used) / total
                let reclaimW = min(usedW, w * Double(reclaim) / total)
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.primary.opacity(0.07))
                    Capsule().fill(Color.primary.opacity(0.22)).frame(width: usedW)
                    if reclaimW > 0 {
                        Capsule().fill(brandGradient)
                            .frame(width: max(reclaimW, 6))
                            .offset(x: usedW - max(reclaimW, 6))
                    }
                }
            }
            .frame(height: 10)
            HStack {
                Text("\(formatBytes(disk.used)) used").foregroundStyle(.secondary)
                Spacer()
                Text("\(formatBytes(disk.free)) free of \(formatBytes(disk.total))").foregroundStyle(.secondary)
            }
            .font(.system(size: 12))
            .monospacedDigit()
        }
    }
}

struct CheckBox: View {
    let state: AppModel.Tri
    var tint: Color = .accentColor

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 5, style: .continuous)
                .strokeBorder(state == .none ? Color.secondary.opacity(0.5) : .clear, lineWidth: 1.5)
                .background(RoundedRectangle(cornerRadius: 5, style: .continuous)
                    .fill(state == .none ? Color.clear : tint))
            if state != .none {
                Image(systemName: state == .all ? "checkmark" : "minus")
                    .font(.system(size: 10, weight: .heavy))
                    .foregroundStyle(.white)
            }
        }
        .frame(width: 18, height: 18)
        .animation(.snappy(duration: 0.15), value: state)
    }
}

// MARK: - Home

struct HomeView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            BrandGlyph(size: 104)
            Text("DeepClean")
                .font(.system(size: 34, weight: .bold, design: .rounded))
                .padding(.top, 22)
            Text("Find gigabytes of caches, build leftovers and junk.")
                .font(.system(size: 14))
                .foregroundStyle(.secondary)
                .padding(.top, 6)

            DiskBar(disk: model.disk)
                .frame(width: 380)
                .padding(.top, 34)

            PrimaryButton(title: "Scan", symbol: "magnifyingglass") { model.scan() }
                .padding(.top, 32)

            if let err = model.errorMessage {
                Label(err, systemImage: "exclamationmark.triangle.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(.orange)
                    .padding(.top, 16)
            }
            Spacer()
            if !model.hasFullDiskAccess {
                Button { model.openFullDiskAccessSettings() } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "lock.open")
                        Text("Allow Full Disk Access to also clean Trash and app sandboxes")
                        Image(systemName: "chevron.right").font(.system(size: 10, weight: .bold))
                    }
                    .font(.system(size: 12, weight: .medium))
                    .padding(.horizontal, 14).padding(.vertical, 8)
                    .background(Capsule().fill(Color.primary.opacity(0.05)))
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .padding(.bottom, 26)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// MARK: - Scanning

struct ScanningView: View {
    @EnvironmentObject var model: AppModel

    private let steps: [(String, String)] = [
        ("caches", "Caches & developer tools"), ("projects", "Project builds"),
        ("downloads", "Downloads"), ("large", "Large files & duplicates"), ("leftovers", "App leftovers"),
    ]

    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            ZStack {
                Circle().stroke(Color.primary.opacity(0.06), lineWidth: 10)
                // Rotation comes from the clock, not a repeatForever animation,
                // which can leak into the screen transition and stall it.
                TimelineView(.animation) { tl in
                    let t = tl.date.timeIntervalSinceReferenceDate
                    Circle()
                        .trim(from: 0, to: 0.28)
                        .stroke(brandGradient, style: StrokeStyle(lineWidth: 10, lineCap: .round))
                        .rotationEffect(.degrees(t.truncatingRemainder(dividingBy: 1) * 360))
                }
                VStack(spacing: 4) {
                    Text(model.scannedFiles.formatted())
                        .font(.system(size: 26, weight: .bold, design: .rounded))
                        .monospacedDigit()
                        .contentTransition(.numericText())
                    Text("items examined").font(.system(size: 12)).foregroundStyle(.secondary)
                }
            }
            .frame(width: 170, height: 170)

            VStack(alignment: .leading, spacing: 12) {
                ForEach(steps, id: \.0) { id, label in
                    let done = model.phasesDone.contains(id)
                    HStack(spacing: 10) {
                        ZStack {
                            if done {
                                Image(systemName: "checkmark.circle.fill")
                                    .foregroundStyle(.green)
                                    .transition(.scale.combined(with: .opacity))
                            } else {
                                ProgressView().controlSize(.small)
                            }
                        }
                        .frame(width: 18, height: 18)
                        Text(label).foregroundStyle(done ? .primary : .secondary)
                    }
                    .font(.system(size: 13))
                }
            }
            .padding(.top, 36)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// MARK: - Results

struct ResultsView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            header
                .padding(.horizontal, 28)
                .padding(.top, 34)
                .padding(.bottom, 18)
            Divider().opacity(0.6)
            ScrollView {
                LazyVStack(spacing: 10) {
                    if !model.hasFullDiskAccess { fdaBanner }
                    ForEach(model.visibleGroups, id: \.id) { g in
                        GroupCard(info: g)
                    }
                }
                .padding(.horizontal, 24)
                .padding(.vertical, 16)
            }
            footer
        }
    }

    private var header: some View {
        HStack(alignment: .bottom, spacing: 24) {
            VStack(alignment: .leading, spacing: 2) {
                Text(formatBytes(model.selectedBytes))
                    .font(.system(size: 44, weight: .bold, design: .rounded))
                    .monospacedDigit()
                    .contentTransition(.numericText())
                    .animation(.snappy, value: model.selectedBytes)
                Text("selected · \(formatBytes(model.foundBytes)) found in \(String(format: "%.1f", model.scanSeconds))s")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
            }
            Spacer()
            DiskBar(disk: model.disk, reclaim: model.selectedBytes)
                .frame(width: 280)
                .padding(.bottom, 4)
        }
    }

    private var fdaBanner: some View {
        Button { model.openFullDiskAccessSettings() } label: {
            HStack(spacing: 8) {
                Image(systemName: "lock.open")
                Text("Allow Full Disk Access to also clean Trash, app sandboxes and leftovers")
                Spacer(minLength: 8)
                Text("Open Settings")
                Image(systemName: "chevron.right").font(.system(size: 10, weight: .bold))
            }
            .font(.system(size: 12, weight: .medium))
            .padding(.horizontal, 16).padding(.vertical, 10)
            .background(Capsule().fill(Color.primary.opacity(0.05)))
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
    }

    private var footer: some View {
        HStack(spacing: 16) {
            Button { model.goHome() } label: {
                Label("Start over", systemImage: "arrow.counterclockwise")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .font(.system(size: 13))

            Spacer()
            if model.selectedAdminCount > 0 {
                Label("Asks for your password", systemImage: "lock.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
            }
            PrimaryButton(title: model.selectedItems.isEmpty ? "Nothing selected" : "Clean \(formatBytes(model.selectedBytes))",
                          symbol: "sparkles",
                          disabled: model.selectedItems.isEmpty) { model.clean() }
        }
        .padding(.horizontal, 28)
        .padding(.vertical, 16)
        .background(.bar)
        .overlay(Divider(), alignment: .top)
    }
}

struct GroupCard: View {
    @EnvironmentObject var model: AppModel
    let info: GroupInfo
    @State private var hovering = false

    var body: some View {
        let open = model.expanded.contains(info.id)
        let items = model.items(in: info.id)
        VStack(spacing: 0) {
            HStack(spacing: 14) {
                Button { model.toggleGroup(info.id) } label: {
                    CheckBox(state: model.state(of: info.id), tint: info.tint)
                }
                .buttonStyle(.plain)

                ZStack {
                    Circle().fill(info.tint.opacity(0.13))
                    Image(systemName: info.symbol)
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(info.tint)
                }
                .frame(width: 36, height: 36)

                VStack(alignment: .leading, spacing: 2) {
                    Text(info.title).font(.system(size: 14, weight: .semibold))
                    Text(info.subtitle).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer()
                VStack(alignment: .trailing, spacing: 2) {
                    let sel = model.bytes(in: info.id, selectedOnly: true)
                    Text(formatBytes(model.bytes(in: info.id)))
                        .font(.system(size: 14, weight: .semibold, design: .rounded))
                        .monospacedDigit()
                    Group {
                        switch model.state(of: info.id) {
                        case .all: Text("all \(items.count) selected")
                        case .some: Text("\(formatBytes(sel)) selected")
                        case .none: Text("\(items.count) item\(items.count == 1 ? "" : "s")")
                        }
                    }
                    .font(.system(size: 11))
                    .foregroundStyle(model.state(of: info.id) == .none ? Color.secondary : info.tint)
                    .monospacedDigit()
                    .contentTransition(.numericText())
                }
                Image(systemName: "chevron.right")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(.tertiary)
                    .rotationEffect(.degrees(open ? 90 : 0))
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
            .onTapGesture {
                withAnimation(.snappy(duration: 0.25)) {
                    if open { model.expanded.remove(info.id) } else { model.expanded.insert(info.id) }
                }
            }

            if open {
                Divider().padding(.leading, 16)
                LazyVStack(spacing: 0) {
                    ForEach(items) { item in
                        ItemRow(item: item, tint: info.tint)
                    }
                }
                .padding(.vertical, 4)
            }
        }
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Color(nsColor: .controlBackgroundColor))
                .shadow(color: .black.opacity(hovering ? 0.08 : 0.04), radius: hovering ? 8 : 4, y: 1)
        )
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Color.primary.opacity(0.06)))
        .onHover { hovering = $0 }
        .animation(.easeOut(duration: 0.15), value: hovering)
    }
}

struct ItemRow: View {
    @EnvironmentObject var model: AppModel
    let item: ScanItem
    let tint: Color
    @State private var hovering = false

    var body: some View {
        let on = model.selected.contains(item.id)
        HStack(spacing: 12) {
            CheckBox(state: on ? .all : .none, tint: tint)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 5) {
                    Text(item.name).font(.system(size: 13)).lineLimit(1).truncationMode(.middle)
                    if item.admin {
                        Image(systemName: "lock.fill").font(.system(size: 9)).foregroundStyle(.secondary)
                    }
                    if item.note.contains("NOT regenerable") || item.note.hasPrefix("duplicate") {
                        Text(item.note.hasPrefix("duplicate") ? "DUPLICATE" : "KEEP?")
                            .font(.system(size: 9, weight: .bold))
                            .padding(.horizontal, 5).padding(.vertical, 1)
                            .background(Capsule().fill((item.note.hasPrefix("duplicate") ? Color.green : Color.orange).opacity(0.15)))
                            .foregroundStyle(item.note.hasPrefix("duplicate") ? .green : .orange)
                    }
                }
                Text([item.category, item.note].filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 12)
            Text(item.size.map(formatBytes) ?? "—")
                .font(.system(size: 12, weight: .medium, design: .rounded))
                .monospacedDigit()
                .foregroundStyle(on ? .primary : .secondary)
        }
        .padding(.leading, 18)
        .padding(.trailing, 41)
        .padding(.vertical, 6)
        .background(hovering ? Color.primary.opacity(0.035) : .clear)
        .contentShape(Rectangle())
        .onTapGesture { model.toggle(item) }
        .onHover { hovering = $0 }
        .contextMenu {
            if item.paths?.isEmpty == false {
                Button("Reveal in Finder") { model.reveal(item) }
            }
            if item.paths?.count == 1 {
                Button("Never Clean This") { model.neverClean(item) }
            }
        }
        .help(item.paths?.first ?? item.note)
    }
}

// MARK: - Cleaning & Done

struct CleaningView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        let fraction = min(1, Double(model.cleanDone) / Double(max(model.cleanTotal, 1)))
        VStack(spacing: 0) {
            Spacer()
            ZStack {
                Circle().stroke(Color.primary.opacity(0.06), lineWidth: 12)
                Circle()
                    .trim(from: 0, to: max(0.02, fraction))
                    .stroke(brandGradient, style: StrokeStyle(lineWidth: 12, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                    .animation(.smooth, value: fraction)
                VStack(spacing: 4) {
                    Text(formatBytes(model.cleanDone))
                        .font(.system(size: 26, weight: .bold, design: .rounded))
                        .monospacedDigit()
                        .contentTransition(.numericText())
                    Text("\(model.filesRemoved.formatted()) files removed")
                        .font(.system(size: 12)).foregroundStyle(.secondary).monospacedDigit()
                }
            }
            .frame(width: 180, height: 180)
            Text(model.cleaningLabel)
                .font(.system(size: 14))
                .foregroundStyle(.secondary)
                .padding(.top, 28)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct DoneView: View {
    @EnvironmentObject var model: AppModel
    @State private var pop = false
    @State private var showProblems = false

    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            ZStack {
                Circle().fill(Color.green.opacity(0.12)).frame(width: 130, height: 130)
                    .scaleEffect(pop ? 1 : 0.6)
                Image(systemName: "checkmark")
                    .font(.system(size: 54, weight: .bold))
                    .foregroundStyle(.green)
                    .scaleEffect(pop ? 1 : 0.3)
            }
            .onAppear { withAnimation(.spring(response: 0.45, dampingFraction: 0.55)) { pop = true } }

            Text("Freed \(formatBytes(model.freed))")
                .font(.system(size: 34, weight: .bold, design: .rounded))
                .padding(.top, 26)
            HStack(spacing: 8) {
                Text(formatBytes(model.freeBefore))
                Image(systemName: "arrow.right").font(.system(size: 11, weight: .bold))
                Text("\(formatBytes(model.disk.free)) free").fontWeight(.semibold).foregroundStyle(.primary)
            }
            .font(.system(size: 14))
            .foregroundStyle(.secondary)
            .monospacedDigit()
            .padding(.top, 8)

            if model.disk.free < model.freeBefore + model.freed / 2 {
                Text("macOS may take a few minutes to release space held by local snapshots.")
                    .font(.system(size: 12)).foregroundStyle(.tertiary).padding(.top, 10)
            }

            if !model.problems.isEmpty {
                Button { withAnimation { showProblems.toggle() } } label: {
                    Label("\(model.problems.count) item\(model.problems.count == 1 ? "" : "s") couldn't be fully cleaned",
                          systemImage: "info.circle")
                        .font(.system(size: 12))
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .padding(.top, 18)
                if showProblems {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 4) {
                            ForEach(model.problems, id: \.self) { Text($0).font(.system(size: 11)).foregroundStyle(.secondary) }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(width: 440, height: 90)
                    .padding(10)
                    .background(RoundedRectangle(cornerRadius: 10).fill(Color.primary.opacity(0.04)))
                    .padding(.top, 8)
                }
            }

            PrimaryButton(title: "Done") { model.goHome() }
                .padding(.top, 34)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// MARK: - Root

struct ContentView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        ZStack {
            LinearGradient(colors: [Color.blue.opacity(0.06), Color.clear], startPoint: .top, endPoint: .center)
                .ignoresSafeArea()
            Group {
                switch model.phase {
                case .home: HomeView()
                case .scanning: ScanningView()
                case .results: ResultsView()
                case .cleaning: CleaningView()
                case .done: DoneView()
                }
            }
            .transition(.opacity.combined(with: .scale(scale: 0.98)))
        }
        .frame(minWidth: 760, minHeight: 600)
        .background(Color(nsColor: .windowBackgroundColor))
    }
}
