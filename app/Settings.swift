import AppKit
import SwiftUI

/// Mirrors src/settings.rs; stored in ~/.config/deepclean/settings.json.
struct EngineSettings: Codable, Equatable {
    var trashPersonal = true
    var projectMinAgeDays = 7
    var largeFileMb = 200
    var oldDownloadDays = 90
    var scanLargeFiles = true
    var scanLeftovers = true

    init() {}

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let d = EngineSettings()
        trashPersonal = try c.decodeIfPresent(Bool.self, forKey: .trashPersonal) ?? d.trashPersonal
        projectMinAgeDays = try c.decodeIfPresent(Int.self, forKey: .projectMinAgeDays) ?? d.projectMinAgeDays
        largeFileMb = try c.decodeIfPresent(Int.self, forKey: .largeFileMb) ?? d.largeFileMb
        oldDownloadDays = try c.decodeIfPresent(Int.self, forKey: .oldDownloadDays) ?? d.oldDownloadDays
        scanLargeFiles = try c.decodeIfPresent(Bool.self, forKey: .scanLargeFiles) ?? d.scanLargeFiles
        scanLeftovers = try c.decodeIfPresent(Bool.self, forKey: .scanLeftovers) ?? d.scanLeftovers
    }
}

@MainActor
final class SettingsStore: ObservableObject {
    static let shared = SettingsStore()

    private let dir = URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(".config/deepclean")
    private var settingsURL: URL { dir.appendingPathComponent("settings.json") }
    private var whitelistURL: URL { dir.appendingPathComponent("whitelist") }

    @Published var settings = EngineSettings() {
        didSet { if settings != oldValue { save() } }
    }
    @Published private(set) var whitelist: [String] = []

    private init() {
        let dec = JSONDecoder()
        dec.keyDecodingStrategy = .convertFromSnakeCase
        if let data = try? Data(contentsOf: settingsURL), let s = try? dec.decode(EngineSettings.self, from: data) {
            settings = s
        }
        loadWhitelist()
    }

    private func save() {
        let enc = JSONEncoder()
        enc.keyEncodingStrategy = .convertToSnakeCase
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? enc.encode(settings).write(to: settingsURL)
    }

    func loadWhitelist() {
        let text = (try? String(contentsOf: whitelistURL, encoding: .utf8)) ?? ""
        whitelist = text.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty && !$0.hasPrefix("#") }
    }

    private func saveWhitelist() {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? (whitelist.joined(separator: "\n") + (whitelist.isEmpty ? "" : "\n"))
            .write(to: whitelistURL, atomically: true, encoding: .utf8)
    }

    func addToWhitelist(_ path: String) {
        guard !whitelist.contains(path) else { return }
        whitelist.append(path)
        saveWhitelist()
    }

    func removeFromWhitelist(_ path: String) {
        whitelist.removeAll { $0 == path }
        saveWhitelist()
    }

    func chooseFolderToProtect() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = true
        panel.allowsMultipleSelection = true
        panel.prompt = "Protect"
        panel.message = "DeepClean will never clean these."
        if panel.runModal() == .OK {
            panel.urls.forEach { addToWhitelist($0.path) }
        }
    }
}

struct SettingsView: View {
    @ObservedObject private var store = SettingsStore.shared
    @AppStorage("showMenuBarExtra") private var showMenuBar = true
    @State private var selection: String?

    var body: some View {
        TabView {
            Form {
                Section {
                    Toggle("Move my own files to the Trash instead of deleting them", isOn: $store.settings.trashPersonal)
                    Text("Applies to downloads, large files, app leftovers and uninstalled apps. Caches are always deleted.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Section("Scan") {
                    Toggle("Look for large files and duplicates", isOn: $store.settings.scanLargeFiles)
                    Stepper("Large files start at \(store.settings.largeFileMb) MB",
                            value: $store.settings.largeFileMb, in: 50...5000, step: 50)
                        .disabled(!store.settings.scanLargeFiles)
                    Stepper("Old downloads: untouched for \(store.settings.oldDownloadDays) days",
                            value: $store.settings.oldDownloadDays, in: 7...730, step: 7)
                    Stepper("Skip projects changed in the last \(store.settings.projectMinAgeDays) days",
                            value: $store.settings.projectMinAgeDays, in: 0...90)
                    Toggle("Look for leftovers from uninstalled apps", isOn: $store.settings.scanLeftovers)
                }
                Section("General") {
                    Toggle("Show free space in the menu bar", isOn: $showMenuBar)
                }
            }
            .formStyle(.grouped)
            .tabItem { Label("General", systemImage: "gearshape") }

            VStack(alignment: .leading, spacing: 10) {
                Text("DeepClean never touches these files and folders.")
                    .font(.callout).foregroundStyle(.secondary)
                List(selection: $selection) {
                    ForEach(store.whitelist, id: \.self) { path in
                        Label(path.replacingOccurrences(of: NSHomeDirectory(), with: "~"), systemImage: "lock.fill")
                            .lineLimit(1).truncationMode(.middle)
                    }
                }
                .listStyle(.bordered(alternatesRowBackgrounds: true))
                .overlay {
                    if store.whitelist.isEmpty {
                        Text("Nothing protected yet").foregroundStyle(.tertiary)
                    }
                }
                HStack(spacing: 8) {
                    SoftButton(title: "Protect…", symbol: "plus") { store.chooseFolderToProtect() }
                    SoftButton(title: "Remove", symbol: "minus", tint: .red, disabled: selection == nil) {
                        if let s = selection { store.removeFromWhitelist(s) }
                    }
                    Spacer()
                }
            }
            .padding(20)
            .onAppear { store.loadWhitelist() }
            .tabItem { Label("Protected", systemImage: "lock.shield") }
        }
        .frame(width: 520, height: 400)
    }
}
