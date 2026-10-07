//! App uninstaller: lists installed apps and finds every file an app spreads
//! around the system (support data, caches, preferences, launch agents, …).

use crate::fsutil::{self, InodeSet};
use crate::model::{Action, Target, group};
use rayon::prelude::*;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone)]
pub struct AppInfo {
    pub name: String,
    pub path: PathBuf,
    pub bundle_id: String,
    pub version: String,
    pub size: u64,
    /// Removing it needs an administrator password (root-owned bundle).
    pub admin: bool,
}

struct Bundle {
    id: String,
    /// What Finder shows (the bundle's file name).
    display: String,
    /// CFBundleName, which is what apps usually name their folders after.
    name: String,
    version: String,
}

fn read_bundle(app: &Path) -> Option<Bundle> {
    let v = plist::Value::from_file(app.join("Contents/Info.plist")).ok()?;
    let d = v.as_dictionary()?;
    let get = |k: &str| d.get(k).and_then(|x| x.as_string()).map(str::to_string);
    let file_name = app.file_stem()?.to_string_lossy().to_string();
    Some(Bundle {
        id: get("CFBundleIdentifier")?,
        display: file_name.clone(),
        name: get("CFBundleName").filter(|n| !n.is_empty()).unwrap_or(file_name),
        version: get("CFBundleShortVersionString").or_else(|| get("CFBundleVersion")).unwrap_or_default(),
    })
}

fn writable(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else { return false };
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// Third-party apps in /Applications and ~/Applications (one folder deep).
pub fn list(home: &Path) -> Vec<AppInfo> {
    let mut bundles = Vec::new();
    for root in [PathBuf::from("/Applications"), home.join("Applications")] {
        let Ok(rd) = fs::read_dir(&root) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "app") {
                bundles.push(p);
            } else if e.file_type().is_ok_and(|t| t.is_dir())
                && let Ok(sub) = fs::read_dir(&p)
            {
                bundles.extend(sub.flatten().map(|s| s.path()).filter(|s| s.extension().is_some_and(|x| x == "app")));
            }
        }
    }
    let seen = InodeSet::default();
    let mut apps: Vec<AppInfo> = bundles
        .par_iter()
        .filter_map(|p| {
            let b = read_bundle(p)?;
            if b.id.starts_with("com.apple.") && !p.to_string_lossy().contains("Install macOS") {
                return None; // system apps can't be removed
            }
            Some(AppInfo {
                name: b.display,
                path: p.clone(),
                bundle_id: b.id,
                version: b.version,
                size: fsutil::disk_usage(p, &seen),
                admin: !writable(p) || !writable(p.parent().unwrap_or(Path::new("/"))),
            })
        })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// The app bundle plus everything it left in ~/Library and /Library.
pub fn related(home: &Path, app: &Path) -> Vec<Target> {
    let Some(b) = read_bundle(app) else { return Vec::new() };
    let id = b.id.as_str();
    let name = b.name.as_str();
    let lib = home.join("Library");
    let mut found: Vec<(&'static str, PathBuf, bool)> = Vec::new(); // (kind, path, admin)

    // exact names
    let exact: &[(&str, &str)] = &[
        ("Support files", "Application Support"),
        ("Caches", "Caches"),
        ("Logs", "Logs"),
        ("Web data", "HTTPStorages"),
        ("Web data", "WebKit"),
        ("Scripts", "Application Scripts"),
    ];
    let display = b.display.as_str();
    for (kind, dir) in exact {
        for n in [id, name, display] {
            found.push((kind, lib.join(dir).join(n), false));
        }
    }
    // Vendor folders, e.g. Application Support/Google/Chrome for com.google.Chrome
    if let Some(vendor) = id.split('.').nth(1) {
        for dir in ["Application Support", "Caches", "Logs"] {
            let Ok(rd) = fs::read_dir(lib.join(dir)) else { continue };
            for e in rd.flatten() {
                if !e.file_name().to_string_lossy().eq_ignore_ascii_case(vendor) {
                    continue;
                }
                for n in [name, display] {
                    found.push(("Support files", e.path().join(n), false));
                }
            }
        }
    }
    found.push(("Saved state", lib.join("Saved Application State").join(format!("{id}.savedState")), false));
    found.push(("Preferences", lib.join("Preferences").join(format!("{id}.plist")), false));
    found.push(("Web data", lib.join("HTTPStorages").join(format!("{id}.binarycookies")), false));
    found.push(("Web data", lib.join("Cookies").join(format!("{id}.binarycookies")), false));
    for (kind, dir) in [("Support files", "/Library/Application Support"), ("Caches", "/Library/Caches")] {
        for n in [id, name] {
            found.push((kind, Path::new(dir).join(n), true));
        }
    }
    found.push(("Preferences", PathBuf::from(format!("/Library/Preferences/{id}.plist")), true));

    // prefix / pattern matches inside a folder
    let scan = |dir: &Path, admin: bool, kind: &'static str, pred: &dyn Fn(&str) -> bool| {
        fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| pred(&e.file_name().to_string_lossy()))
                    .map(|e| (kind, e.path(), admin))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let dotted = format!("{id}.");
    found.extend(scan(&lib.join("Containers"), false, "Sandbox", &|n| n == id || n.starts_with(&dotted)));
    found.extend(scan(&lib.join("Group Containers"), false, "Sandbox", &|n| {
        n.ends_with(&format!(".{id}")) || n == format!("group.{id}")
    }));
    found.extend(scan(&lib.join("Preferences"), false, "Preferences", &|n| n.starts_with(&dotted) && n.ends_with(".plist")));
    found.extend(scan(&lib.join("Preferences/ByHost"), false, "Preferences", &|n| n.starts_with(&dotted)));
    found.extend(scan(&lib.join("LaunchAgents"), false, "Launch agents", &|n| n.starts_with(id)));
    for dir in ["/Library/LaunchAgents", "/Library/LaunchDaemons", "/Library/PrivilegedHelperTools"] {
        found.extend(scan(Path::new(dir), true, "Helpers", &|n| n.starts_with(id)));
    }

    let mut seen_paths = std::collections::HashSet::new();
    let found: Vec<_> = found
        .into_iter()
        .filter(|(_, p, _)| fs::symlink_metadata(p).is_ok() && seen_paths.insert(p.clone()))
        .collect();

    let mut out = vec![
        Target::new(group::UNINSTALL, "Application", format!("{display}.app"), Action::Delete { paths: vec![app.to_path_buf()] })
            .note(b.version.clone())
            .admin(!writable(app) || !writable(app.parent().unwrap_or(Path::new("/"))))
            .trash(true),
    ];
    for (kind, p, admin) in found {
        let shown = fsutil::tilde(&p, home);
        out.push(
            Target::new(group::UNINSTALL, kind, shown, Action::Delete { paths: vec![p] })
                .admin(admin)
                .trash(true),
        );
    }
    let seen = InodeSet::default();
    out.par_iter_mut().for_each(|t| t.size = Some(t.paths().iter().map(|p| fsutil::disk_usage(p, &seen)).sum()));
    out
}

/// Find an installed app by (case-insensitive) name or path.
pub fn find(home: &Path, query: &str) -> Option<AppInfo> {
    let q = query.trim_end_matches(".app").to_lowercase();
    let apps = list(home);
    apps.iter()
        .find(|a| a.name.to_lowercase() == q || a.path.to_string_lossy().to_lowercase() == query.to_lowercase())
        .or_else(|| apps.iter().find(|a| a.name.to_lowercase().contains(&q)))
        .cloned()
}
