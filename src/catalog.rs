//! The catalog of regenerable caches, logs and junk DeepClean knows about.
//!
//! Patterns are globs; `~/` expands to the user's home. A pattern names the
//! thing to delete — use `dir/*` to empty a directory while keeping it.

use crate::fsutil::age_days;
use crate::model::{Action, Target, group};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Spec {
    category: &'static str,
    name: &'static str,
    patterns: Vec<String>,
    default_on: bool,
    /// Broad sweeps (e.g. `~/Library/Caches/*`) yield to more specific entries,
    /// and empty each matched folder rather than removing it (apps and daemons
    /// expect their cache folder to exist).
    broad: bool,
    /// Child names a broad sweep must never touch.
    exclude: &'static [&'static str],
    /// Only delete matches untouched for this many days.
    min_age_days: u64,
    /// Process names; if any is running the entry is deselected by default.
    procs: &'static [&'static str],
    note: &'static str,
    admin: bool,
    /// Reads other apps' sandboxes: needs Full Disk Access (or macOS prompts).
    containers: bool,
}

fn spec(category: &'static str, name: &'static str, patterns: &[&str], default_on: bool) -> Spec {
    Spec {
        category,
        name,
        patterns: patterns.iter().map(|s| s.to_string()).collect(),
        default_on,
        broad: false,
        exclude: &[],
        min_age_days: 0,
        procs: &[],
        note: "",
        admin: false,
        containers: false,
    }
}

impl Spec {
    fn broad(mut self, exclude: &'static [&'static str]) -> Self {
        self.broad = true;
        self.exclude = exclude;
        self
    }
    fn older_than(mut self, days: u64) -> Self {
        self.min_age_days = days;
        self
    }
    fn procs(mut self, p: &'static [&'static str]) -> Self {
        self.procs = p;
        self
    }
    fn note(mut self, n: &'static str) -> Self {
        self.note = n;
        self
    }
    fn admin(mut self) -> Self {
        self.admin = true;
        self
    }
    fn containers(mut self) -> Self {
        self.containers = true;
        self
    }
}

/// Cache directories Chromium creates inside each browser profile.
fn chromium(name: &'static str, base: &str, procs: &'static [&'static str]) -> Spec {
    let mut pats = Vec::new();
    for sub in [
        "Cache", "Code Cache", "GPUCache", "DawnCache", "DawnGraphiteCache", "DawnWebGPUCache",
        "Service Worker/CacheStorage", "Service Worker/ScriptCache",
    ] {
        pats.push(format!("{base}/*/{sub}"));
    }
    for sub in [
        "GrShaderCache", "ShaderCache", "GraphiteDawnCache", "component_crx_cache",
        "extensions_crx_cache", "Crashpad/completed",
    ] {
        pats.push(format!("{base}/{sub}"));
    }
    let mut s = spec("Browsers", name, &[], true).procs(procs);
    s.patterns = pats;
    s
}

fn specs() -> Vec<Spec> {
    let tmp = std::env::temp_dir();
    let tmp = tmp.to_string_lossy().trim_end_matches('/').to_string();
    vec![
        // ── User essentials ─────────────────────────────────────────────
        spec("System", "App caches", &["~/Library/Caches/*"], true).broad(&[
            "CloudKit", "com.apple.bird", "com.apple.HomeKit", "com.apple.homed",
            "com.apple.containermanagerd", "com.apple.nsurlsessiond", "FamilyCircle",
            "com.apple.ap.adprivacyd", "deepclean",
        ]),
        spec("System", "User logs & crash reports",
             &["~/Library/Logs/*", "~/Library/Application Support/CrashReporter/*"], true)
            .broad(&["deepclean"]),
        spec("System", "Trash", &["~/.Trash/*", "/Volumes/*/.Trashes/%UID%/*"], true),
        spec("System", "Old temporary files", &[&format!("{tmp}/*")], false)
            .older_than(7)
            .note("untouched for 7+ days"),
        spec("System", "Saved window state", &["~/Library/Saved Application State/*"], false),
        spec("System", "Developer tool caches (~/.cache)", &["~/.cache/*"], true).broad(&[
            "huggingface", "lm-studio", "torch", "whisper", "deepclean", "claude", "codex",
            "opencode", "gemini",
        ]),
        spec("System", "Sandboxed app caches", &[
            "~/Library/Containers/*/Data/Library/Caches/*",
            "~/Library/Group Containers/*/Library/Caches/*",
        ], true)
            .containers(),
        spec("System", "Mail attachment downloads",
             &["~/Library/Containers/com.apple.mail/Data/Library/Mail Downloads/*"], true)
            .containers(),
        spec("System", "System caches", &["/Library/Caches/*"], true)
            .admin()
            .broad(&["com.apple.iconservices.store", "com.apple.amsengagementd.classicdatavault"]),
        spec("System", "System logs & diagnostic reports",
             &["/Library/Logs/DiagnosticReports/*", "/private/var/log/*.gz", "/private/var/log/asl/*.asl"], true)
            .admin(),

        // ── Browsers ────────────────────────────────────────────────────
        chromium("Google Chrome", "~/Library/Application Support/Google/Chrome", &["Google Chrome"]),
        chromium("Brave", "~/Library/Application Support/BraveSoftware/Brave-Browser", &["Brave Browser"]),
        chromium("Microsoft Edge", "~/Library/Application Support/Microsoft Edge", &["Microsoft Edge"]),
        chromium("Arc", "~/Library/Application Support/Arc/User Data", &["Arc"]),
        chromium("Vivaldi", "~/Library/Application Support/Vivaldi", &["Vivaldi"]),
        chromium("Opera", "~/Library/Application Support/com.operasoftware.Opera", &["Opera"]),
        chromium("Chromium", "~/Library/Application Support/Chromium", &["Chromium"]),
        spec("Browsers", "Firefox", &[
            "~/Library/Caches/Firefox", "~/Library/Caches/Mozilla",
            "~/Library/Application Support/Firefox/Profiles/*/cache2",
            "~/Library/Application Support/Firefox/Profiles/*/startupCache",
        ], true).procs(&["firefox"]),
        spec("Browsers", "Safari", &["~/Library/Caches/com.apple.Safari"], true).procs(&["Safari"]),

        // ── Apps ────────────────────────────────────────────────────────
        spec("Apps", "Electron & Chromium app caches", &[
            "~/Library/Application Support/*/Cache", "~/Library/Application Support/*/Code Cache",
            "~/Library/Application Support/*/GPUCache", "~/Library/Application Support/*/CachedData",
            "~/Library/Application Support/*/DawnCache", "~/Library/Application Support/*/DawnGraphiteCache",
            "~/Library/Application Support/*/DawnWebGPUCache",
            "~/Library/Application Support/*/Service Worker/CacheStorage",
            "~/Library/Application Support/*/Partitions/*/Cache",
            "~/Library/Application Support/*/Partitions/*/Code Cache",
        ], true).note("Slack, Discord, VS Code, Notion, Teams, …"),
        spec("Apps", "VS Code / Cursor / Windsurf extras", &[
            "~/Library/Application Support/Code/CachedExtensionVSIXs",
            "~/Library/Application Support/Code/logs",
            "~/Library/Application Support/Code - Insiders/CachedExtensionVSIXs",
            "~/Library/Application Support/Code - Insiders/logs",
            "~/Library/Application Support/Cursor/CachedExtensionVSIXs",
            "~/Library/Application Support/Cursor/logs",
            "~/Library/Application Support/Windsurf/CachedExtensionVSIXs",
            "~/Library/Application Support/Windsurf/logs",
        ], true),
        spec("Apps", "Adobe media cache", &[
            "~/Library/Application Support/Adobe/Common/Media Cache Files",
            "~/Library/Application Support/Adobe/Common/Media Cache",
            "~/Library/Application Support/Adobe/Common/Peak Files",
        ], true).note("Premiere, After Effects, Audition"),
        spec("Apps", "Steam download & shader caches", &[
            "~/Library/Application Support/Steam/appcache/httpcache",
            "~/Library/Application Support/Steam/depotcache",
            "~/Library/Application Support/Steam/logs",
            "~/Library/Application Support/Steam/steamapps/shadercache",
            "~/Library/Application Support/Steam/steamapps/downloading",
        ], true).procs(&["steam_osx"]),
        spec("Apps", "Minecraft launcher caches", &[
            "~/Library/Application Support/minecraft/webcache", "~/Library/Application Support/minecraft/webcache2",
            "~/Library/Application Support/minecraft/logs",
        ], true),
        spec("Apps", "iPhone & iPad update files", &["~/Library/iTunes/*Software Updates/*"], true),
        spec("Apps", "iOS device backups", &["~/Library/Application Support/MobileSync/Backup/*"], false)
            .note("NOT regenerable. Your only copy unless you use iCloud Backup"),

        // ── Xcode & Apple dev ───────────────────────────────────────────
        spec("Xcode", "DerivedData", &["~/Library/Developer/Xcode/DerivedData/*"], true).procs(&["Xcode"]),
        spec("Xcode", "Device support files", &["~/Library/Developer/Xcode/*DeviceSupport/*"], true)
            .note("re-created when a device connects"),
        spec("Xcode", "Simulator caches & previews", &[
            "~/Library/Developer/CoreSimulator/Caches/*", "~/Library/Developer/Xcode/UserData/Previews",
        ], true),
        spec("Xcode", "Xcode docs cache, logs & products", &[
            "~/Library/Developer/Xcode/DocumentationCache", "~/Library/Developer/Xcode/UserData/IB Support",
            "~/Library/Developer/Xcode/iOS Device Logs", "~/Library/Developer/Xcode/Products",
            "~/Library/Logs/CoreSimulator",
        ], true),
        spec("Xcode", "Archives", &["~/Library/Developer/Xcode/Archives/*"], false)
            .note("NOT regenerable. Contains dSYMs for shipped builds"),
        spec("Xcode", "SwiftPM caches", &["~/Library/Caches/org.swift.swiftpm", "~/Library/org.swift.swiftpm/cache"], true),

        // ── JavaScript ──────────────────────────────────────────────────
        spec("JavaScript", "npm cache", &["~/.npm/_cacache", "~/.npm/_npx", "~/.npm/_logs", "~/.npm/_prebuilds"], true),
        spec("JavaScript", "Yarn cache", &["~/Library/Caches/Yarn", "~/.yarn/berry/cache", "~/.cache/yarn"], true),
        spec("JavaScript", "pnpm store", &["~/Library/pnpm/store", "~/.local/share/pnpm/store", "~/.pnpm-store"], true),
        spec("JavaScript", "Bun cache", &["~/.bun/install/cache"], true),
        spec("JavaScript", "Deno cache", &["~/Library/Caches/deno"], true),
        spec("JavaScript", "node-gyp, Electron & browser-test caches", &[
            "~/Library/Caches/node-gyp", "~/.node-gyp", "~/Library/Caches/electron",
            "~/Library/Caches/electron-builder", "~/Library/Caches/ms-playwright", "~/.cache/puppeteer",
            "~/Library/Caches/Cypress",
        ], true),

        // ── Python ──────────────────────────────────────────────────────
        spec("Python", "pip, uv, Poetry & pipenv caches", &[
            "~/Library/Caches/pip", "~/.cache/pip", "~/.cache/uv", "~/Library/Caches/uv",
            "~/Library/Caches/pypoetry/cache", "~/Library/Caches/pypoetry/artifacts",
            "~/Library/Caches/pipenv", "~/.cache/pre-commit",
        ], true),
        spec("Python", "Poetry virtualenvs", &["~/Library/Caches/pypoetry/virtualenvs/*"], false)
            .note("re-created by `poetry install`"),
        spec("AI", "Downloaded AI models", &[
            "~/.cache/huggingface", "~/.cache/torch", "~/.cache/whisper", "~/.ollama/models",
            "~/.cache/lm-studio/models", "~/.lmstudio/models",
        ], false).note("large; re-download with `ollama pull` / the app"),

        // ── Rust / Go / native ──────────────────────────────────────────
        spec("Rust", "Cargo registry & git caches", &[
            "~/.cargo/registry/cache", "~/.cargo/registry/src", "~/.cargo/registry/index",
            "~/.cargo/git/checkouts", "~/.cargo/git/db",
        ], true),
        spec("Rust", "rustup downloads", &["~/.rustup/downloads", "~/.rustup/tmp"], true),
        spec("Go", "Go module & build cache", &["~/go/pkg/mod", "~/Library/Caches/go-build", "~/.cache/go-build"], true),
        spec("Native", "ccache, sccache & Bazel", &[
            "~/Library/Caches/ccache", "~/.ccache", "~/.cache/ccache", "~/Library/Caches/Mozilla.sccache",
            "~/.cache/sccache", "~/Library/Caches/bazel", "~/.cache/bazel",
        ], true),
        spec("Native", "Homebrew download cache", &["~/Library/Caches/Homebrew"], true),

        // ── JVM / Android / game engines ────────────────────────────────
        spec("JVM", "Gradle caches & wrappers", &[
            "~/.gradle/caches", "~/.gradle/wrapper/dists", "~/.gradle/daemon", "~/.gradle/native",
            "~/.gradle/.tmp", "~/.gradle/jdks",
        ], true).procs(&["java"]),
        spec("JVM", "MCreator Gradle cache", &[
            "~/.mcreator/gradle/caches", "~/.mcreator/gradle/wrapper/dists", "~/.mcreator/gradle/daemon",
            "~/.mcreator/logs",
        ], true).procs(&["java"]),
        spec("JVM", "Maven repository", &["~/.m2/repository"], false)
            .note("artifacts from `mvn install` are not re-downloadable"),
        spec("JVM", "sbt, Ivy & Coursier", &[
            "~/.ivy2/cache", "~/.sbt/boot", "~/Library/Caches/Coursier", "~/.cache/coursier",
        ], true),
        spec("JVM", "Kotlin/Native & Android caches", &[
            "~/.konan/cache", "~/.konan/dependencies", "~/.android/cache", "~/.android/build-cache",
        ], true),
        spec("JVM", "JetBrains IDE caches", &["~/Library/Caches/JetBrains", "~/Library/Logs/JetBrains"], true),
        spec("Game engines", "Unity & Unreal caches", &[
            "~/Library/Unity/cache", "~/Library/Unity/Asset Store-5.x",
            "~/Library/Application Support/Epic/UnrealEngine/Common/DerivedDataCache",
            "~/Library/Application Support/Godot/shader_cache",
        ], true),

        // ── Other ecosystems ────────────────────────────────────────────
        spec("Other dev", "CocoaPods & Carthage", &[
            "~/Library/Caches/CocoaPods", "~/Library/Caches/org.carthage.CarthageKit",
        ], true),
        spec("Other dev", "Dart & Flutter pub cache", &["~/.pub-cache/hosted", "~/.pub-cache/git", "~/.pub-cache/_temp"], true),
        spec("Other dev", ".NET NuGet packages", &[
            "~/.nuget/packages", "~/.local/share/NuGet/v3-cache", "~/.local/share/NuGet/http-cache",
        ], true),
        spec("Other dev", "Ruby, PHP, Elixir & Haskell caches", &[
            "~/.bundle/cache", "~/.composer/cache", "~/Library/Caches/composer", "~/.hex/cache",
            "~/.mix/archives/cache", "~/.cabal/packages", "~/.stack/pantry",
        ], true),
        spec("Other dev", "Terraform plugin cache", &["~/.terraform.d/plugin-cache"], true),
    ]
}

fn group_for(s: &Spec) -> &'static str {
    match s.category {
        _ if s.admin => group::SYSTEM,
        "System" => group::JUNK,
        "Browsers" => group::BROWSERS,
        "Apps" => group::APPS,
        _ => group::DEV,
    }
}

pub struct ScanOptions {
    /// Include specs that need administrator rights.
    pub admin: bool,
    /// Include specs that read other apps' containers.
    pub containers: bool,
}

fn expand_home(pat: &str, home: &Path) -> String {
    let uid = unsafe { libc::getuid() }.to_string();
    let pat = pat.replace("%UID%", &uid);
    match pat.strip_prefix("~/") {
        Some(rest) => format!("{}/{}", glob::Pattern::escape(&home.to_string_lossy()), rest),
        None => pat,
    }
}

pub fn running_processes() -> HashSet<String> {
    Command::new("/bin/ps")
        .args(["-axco", "comm="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).collect())
        .unwrap_or_default()
}

/// Expand the catalog into concrete targets (sizes not yet measured).
pub fn build_targets(home: &Path, opts: &ScanOptions) -> Vec<Target> {
    let specs: Vec<Spec> = specs()
        .into_iter()
        .filter(|s| opts.admin || !s.admin)
        .filter(|s| opts.containers || !s.containers)
        .collect();
    let running = running_processes();

    // 1. glob-expand every pattern
    let mut matches: Vec<Vec<PathBuf>> = specs
        .iter()
        .map(|s| {
            let mut v = Vec::new();
            for pat in &s.patterns {
                let Ok(paths) = glob::glob(&expand_home(pat, home)) else { continue };
                for p in paths.flatten() {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    if s.exclude.contains(&name.as_str()) || name == ".DS_Store" || name == ".localized" {
                        continue;
                    }
                    if s.min_age_days > 0 && age_days(&p) < s.min_age_days {
                        continue;
                    }
                    v.push(p);
                }
            }
            v
        })
        .collect();

    // 2. broad sweeps give way to any specific entry at or below their match
    let specific: Vec<PathBuf> = specs
        .iter()
        .zip(&matches)
        .filter(|(s, _)| !s.broad)
        .flat_map(|(_, m)| m.iter().cloned())
        .collect();
    for (s, m) in specs.iter().zip(matches.iter_mut()) {
        if s.broad {
            m.retain(|p| !specific.iter().any(|q| q.starts_with(p)));
        }
    }

    // 3. never count a path twice: drop anything already claimed or nested in a claimed path
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    let all: HashSet<PathBuf> = matches.iter().flatten().cloned().collect();
    for m in matches.iter_mut() {
        m.retain(|p| {
            let nested = p.ancestors().skip(1).any(|a| all.contains(a));
            !nested && claimed.insert(p.clone())
        });
    }

    for (s, m) in specs.iter().zip(matches.iter_mut()) {
        if s.broad {
            *m = m.drain(..).flat_map(children_or_self).collect();
        }
    }

    specs
        .into_iter()
        .zip(matches)
        .filter(|(_, m)| !m.is_empty())
        .map(|(s, paths)| {
            let busy: Vec<&str> = s.procs.iter().copied().filter(|p| running.contains(*p)).collect();
            let note = if busy.is_empty() {
                s.note.to_string()
            } else {
                format!("Quit {} first", busy.join(", "))
            };
            Target::new(group_for(&s), s.category, s.name, Action::Delete { paths })
                .on(s.default_on && busy.is_empty())
                .note(note)
                .admin(s.admin)
        })
        .collect()
}

fn children_or_self(p: PathBuf) -> Vec<PathBuf> {
    let is_dir = std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir());
    if !is_dir {
        return vec![p];
    }
    std::fs::read_dir(&p)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}
