import SwiftUI

final class AppDelegate: NSObject, NSApplicationDelegate {
    /// Keep running in the menu bar when the window closes, if that's enabled.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        !UserDefaults.standard.bool(forKey: "showMenuBarExtra")
    }
}

@main
struct DeepCleanApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var model = AppModel()
    @StateObject private var bar = MenuBarModel()
    @AppStorage("showMenuBarExtra") private var showMenuBar = true

    init() {
        UserDefaults.standard.register(defaults: ["showMenuBarExtra": true])
    }

    var body: some Scene {
        Window("DeepClean", id: "main") {
            RootView()
                .environmentObject(model)
                .onAppear { History.shared.reload() }
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: 1060, height: 720)
        .windowResizability(.contentMinSize)
        .commands {
            CommandGroup(replacing: .newItem) {}
            CommandMenu("Clean") {
                Button("Scan") {
                    model.section = .clean
                    model.scan()
                }
                .keyboardShortcut("r")
                .disabled(model.phase == .scanning || model.phase == .cleaning)
            }
            CommandGroup(after: .sidebar) {
                ForEach(Array(SidebarSection.allCases.enumerated()), id: \.offset) { i, s in
                    Button(s.title) { model.section = s }
                        .keyboardShortcut(KeyEquivalent(Character("\(i + 1)")), modifiers: .command)
                }
            }
        }

        Settings {
            SettingsView()
        }

        MenuBarExtra(isInserted: $showMenuBar) {
            MenuBarContent(bar: bar).environmentObject(model)
        } label: {
            Text("\(Image(systemName: "sparkles")) \(bar.label)")
        }
    }
}
