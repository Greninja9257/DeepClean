import AppKit
import SwiftUI

struct HistoryEntry: Identifiable {
    enum Kind { case cleaned, trashed, task }
    let id = UUID()
    let date: Date
    let name: String
    let bytes: UInt64
    let kind: Kind
}

/// Reads ~/Library/Logs/deepclean/operations.log (written by the engine).
@MainActor
final class History: ObservableObject {
    static let shared = History()
    @Published private(set) var entries: [HistoryEntry] = []

    let logURL = URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("Library/Logs/deepclean/operations.log")

    var totalFreed: UInt64 { entries.filter { $0.kind == .cleaned }.reduce(0) { $0 + $1.bytes } }
    var totalTrashed: UInt64 { entries.filter { $0.kind == .trashed }.reduce(0) { $0 + $1.bytes } }

    func reload() {
        let text = (try? String(contentsOf: logURL, encoding: .utf8)) ?? ""
        let iso = ISO8601DateFormatter()
        var out: [HistoryEntry] = []
        for line in text.split(separator: "\n") {
            let f = line.split(separator: "\t", maxSplits: 3).map(String.init)
            guard f.count == 4, let date = iso.date(from: f[0]) else { continue }
            let kind: HistoryEntry.Kind
            switch f[1] {
            case "target": kind = .cleaned
            case "trashed-target": kind = .trashed
            case "command": kind = .task
            default: continue
            }
            let bytes = UInt64(f[2]) ?? 0
            if kind != .task && bytes == 0 { continue }
            out.append(HistoryEntry(date: date, name: f[3], bytes: bytes, kind: kind))
        }
        entries = out.reversed()
    }

    /// Entries grouped by calendar day, newest first.
    var byDay: [(day: Date, items: [HistoryEntry])] {
        let cal = Calendar.current
        let groups = Dictionary(grouping: entries) { cal.startOfDay(for: $0.date) }
        return groups.keys.sorted(by: >).map { ($0, groups[$0]!) }
    }
}

struct HistoryView: View {
    @ObservedObject private var history = History.shared

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .bottom) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(formatBytes(history.totalFreed))
                        .font(.system(size: 40, weight: .bold, design: .rounded))
                        .monospacedDigit()
                    Text(history.totalTrashed > 0
                         ? "freed so far, plus \(formatBytes(history.totalTrashed)) moved to the Trash"
                         : "freed so far")
                        .font(.system(size: 13)).foregroundStyle(.secondary)
                }
                Spacer()
                SoftButton(title: "Show Log", symbol: "doc.text.magnifyingglass") {
                    NSWorkspace.shared.activateFileViewerSelecting([history.logURL])
                }
            }
            .padding(.horizontal, 28).padding(.top, 22).padding(.bottom, 18)
            Divider().opacity(0.6)

            if history.entries.isEmpty {
                ContentUnavailableView("No cleanups yet", systemImage: "clock",
                                       description: Text("Everything DeepClean cleans shows up here."))
            } else {
                List {
                    ForEach(history.byDay, id: \.day) { day in
                        Section(day.day.formatted(date: .complete, time: .omitted)) {
                            ForEach(day.items) { e in
                                HStack(spacing: 10) {
                                    Image(systemName: e.kind == .task ? "wand.and.stars" : e.kind == .trashed ? "trash" : "sparkles")
                                        .foregroundStyle(e.kind == .task ? Color.orange : Color.blue)
                                        .frame(width: 18)
                                    Text(e.name).lineLimit(1).truncationMode(.middle)
                                    Spacer()
                                    Text(e.date.formatted(date: .omitted, time: .shortened))
                                        .foregroundStyle(.tertiary).font(.system(size: 11))
                                    Text(e.bytes > 0 ? formatBytes(e.bytes) : "—")
                                        .font(.system(size: 12, weight: .medium, design: .rounded))
                                        .monospacedDigit()
                                        .frame(width: 80, alignment: .trailing)
                                }
                                .font(.system(size: 13))
                            }
                        }
                    }
                }
                .scrollContentBackground(.hidden)
            }
        }
        .onAppear { history.reload() }
    }
}
