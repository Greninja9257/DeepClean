//! Fast parallel filesystem primitives: disk-usage measurement, recursive
//! deletion, and the path safety gate every deletion must pass.

use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Live counters shown by the spinner / progress bar.
pub static FILES_SEEN: AtomicU64 = AtomicU64::new(0);
pub static BYTES_SEEN: AtomicU64 = AtomicU64::new(0);
pub static FILES_DELETED: AtomicU64 = AtomicU64::new(0);

/// Tracks (dev, inode) pairs of multiply-linked files so hard links
/// (pnpm stores, conda pkgs, ...) are only counted once.
#[derive(Default)]
pub struct InodeSet(Mutex<HashSet<(u64, u64)>>);

impl InodeSet {
    fn first_sighting(&self, md: &fs::Metadata) -> bool {
        if md.nlink() <= 1 {
            return true;
        }
        self.0.lock().unwrap().insert((md.dev(), md.ino()))
    }
}

/// Bytes actually allocated on disk (st_blocks), not the logical length.
fn allocated(md: &fs::Metadata) -> u64 {
    md.blocks() * 512
}

/// Disk usage of `path` (file or directory, never following symlinks).
pub fn disk_usage(path: &Path, seen: &InodeSet) -> u64 {
    match fs::symlink_metadata(path) {
        Ok(md) if md.is_dir() => allocated(&md) + dir_usage(path, seen),
        Ok(md) => {
            FILES_SEEN.fetch_add(1, Ordering::Relaxed);
            if seen.first_sighting(&md) {
                let b = allocated(&md);
                BYTES_SEEN.fetch_add(b, Ordering::Relaxed);
                b
            } else {
                0
            }
        }
        Err(_) => 0,
    }
}

fn dir_usage(dir: &Path, seen: &InodeSet) -> u64 {
    let Ok(rd) = fs::read_dir(dir) else { return 0 };
    let mut total = 0u64;
    let mut files = 0u64;
    let mut subdirs = Vec::new();
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            subdirs.push(entry.path());
        } else if let Ok(md) = entry.metadata() {
            // DirEntry::metadata is an lstat: symlinks are measured, not followed.
            files += 1;
            if seen.first_sighting(&md) {
                total += allocated(&md);
            }
        }
    }
    FILES_SEEN.fetch_add(files, Ordering::Relaxed);
    BYTES_SEEN.fetch_add(total, Ordering::Relaxed);
    total
        + subdirs
            .par_iter()
            .map(|d| {
                let own = fs::symlink_metadata(d).map(|m| allocated(&m)).unwrap_or(0);
                own + dir_usage(d, seen)
            })
            .sum::<u64>()
}

/// Recursively delete `path` in parallel. Symlinks are unlinked, never followed.
/// Read-only directories (e.g. the Go module cache) are made writable first.
/// Returns the number of entries that could not be removed.
pub fn remove_tree(path: &Path) -> u64 {
    let md = match fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return 0,
        Err(_) => return 1,
    };
    if !md.is_dir() {
        return match unlink(path) {
            Ok(()) => 0,
            Err(_) => 1,
        };
    }
    ensure_writable(path, &md);
    let Ok(rd) = fs::read_dir(path) else { return 1 };
    let mut failures = 0u64;
    let mut subdirs = Vec::new();
    for entry in rd.flatten() {
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => subdirs.push(entry.path()),
            _ => {
                if unlink(&entry.path()).is_err() {
                    failures += 1;
                }
            }
        }
    }
    failures += subdirs.par_iter().map(|d| remove_tree(d)).sum::<u64>();
    if fs::remove_dir(path).is_err() {
        failures += 1;
    }
    failures
}

fn unlink(p: &Path) -> io::Result<()> {
    fs::remove_file(p)?;
    FILES_DELETED.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

fn ensure_writable(dir: &Path, md: &fs::Metadata) {
    let mode = md.permissions().mode();
    if mode & 0o700 != 0o700 {
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(mode | 0o700));
    }
}

/// Days since the path was last modified (0 if unknown or in the future).
pub fn age_days(path: &Path) -> u64 {
    fs::symlink_metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .unwrap_or(Duration::ZERO)
        .as_secs()
        / 86_400
}

/// Free bytes available to the user on the volume containing `path`.
pub fn free_space(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some(s.f_bavail * s.f_bsize as u64)
}

/// Total size of the volume containing `path`.
pub fn total_space(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some(s.f_blocks * s.f_bsize as u64)
}

pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Shorten a path for display by replacing the home prefix with `~`.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// A third-party app bundle directly in /Applications (or one folder down).
/// Apple's own apps are never removable.
fn is_removable_app(path: &Path) -> bool {
    let comps: Vec<_> = path.components().collect();
    let in_apps = path.starts_with("/Applications") && (comps.len() == 3 || comps.len() == 4);
    if !in_apps || path.extension().is_none_or(|e| e != "app") {
        return false;
    }
    let id = plist::Value::from_file(path.join("Contents/Info.plist"))
        .ok()
        .and_then(|v| v.as_dictionary()?.get("CFBundleIdentifier")?.as_string().map(str::to_string));
    // Unreadable bundles (e.g. installer apps without a plist) are allowed;
    // anything identifying as Apple's is not.
    !id.is_some_and(|i| i.starts_with("com.apple."))
        || path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Install macOS"))
}

/// Move `path` into the Trash of the volume it lives on, renaming on
/// collision the way Finder does ("name 2", "name 3", …).
pub fn move_to_trash(path: &Path, home: &Path) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| io::Error::other("no file name"))?;
    let comps: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    let trash = if comps.len() > 3 && comps[1] == "Volumes" {
        let uid = fs::metadata(home).map(|m| m.uid()).unwrap_or_else(|_| unsafe { libc::getuid() });
        PathBuf::from("/Volumes").join(&comps[2]).join(".Trashes").join(uid.to_string())
    } else {
        home.join(".Trash")
    };
    let _ = fs::create_dir_all(&trash);
    // Like Finder: "photo 2.jpg" and "Foo 2.app", but "com.vendor.app 2" for folders.
    let is_plain_dir = fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
        && path.extension().is_none_or(|e| e != "app" && e != "bundle");
    let (stem, ext) = if is_plain_dir {
        (name.to_string_lossy().to_string(), String::new())
    } else {
        (
            Path::new(name).file_stem().unwrap_or(name).to_string_lossy().to_string(),
            Path::new(name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default(),
        )
    };
    // RENAME_EXCL makes the rename fail instead of replacing an existing item,
    // so parallel moves of same-named items can never overwrite each other.
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let from = CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
    for n in 1.. {
        let dest = if n == 1 { trash.join(name) } else { trash.join(format!("{stem} {n}{ext}")) };
        let to = CString::new(dest.as_os_str().as_bytes()).map_err(io::Error::other)?;
        if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } == 0 {
            FILES_DELETED.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EEXIST) || n > 10_000 {
            return Err(err);
        }
    }
    unreachable!()
}

/// The last line of defence: refuse anything that isn't clearly a disposable
/// location. Every path passes through here immediately before deletion.
pub fn is_safe_to_delete(path: &Path, home: &Path, whitelist: &[PathBuf]) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("not an absolute path".into());
    }
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err("path is not normalized".into());
    }
    let app_bundle = is_removable_app(path);
    if path.components().count() < 4 && !app_bundle {
        return Err("path is too shallow".into());
    }
    const PROTECTED_HOME: &[&str] = &[
        "",
        "Library",
        "Library/Caches",
        "Library/Logs",
        "Library/Application Support",
        "Library/Containers",
        "Library/Developer",
        "Library/Mobile Documents",
        "Desktop",
        "Documents",
        "Downloads",
        "Pictures",
        "Movies",
        "Music",
        "Public",
        "Applications",
        ".ssh",
        ".gnupg",
        ".config",
        ".cache",
        ".cargo",
        ".rustup",
        ".gradle",
        ".m2",
        ".npm",
    ];
    if PROTECTED_HOME.iter().any(|p| path == home.join(p)) {
        return Err("protected directory".into());
    }
    for sensitive in [
        ".ssh",
        ".gnupg",
        "Library/Keychains",
        "Library/Mobile Documents",
    ] {
        if path.starts_with(home.join(sensitive)) {
            return Err("inside a sensitive directory".into());
        }
    }
    let tmp = std::env::temp_dir();
    let allowed_roots = [
        home.to_path_buf(),
        PathBuf::from("/Library/Caches"),
        PathBuf::from("/Library/Logs"),
        PathBuf::from("/private/var/folders"),
        tmp,
    ];
    let comps: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let uid = unsafe { libc::getuid() }.to_string();
    // /Volumes/<disk>/.Trashes/<uid>/<item>
    let external_trash = comps.len() >= 6
        && comps[1] == "Volumes"
        && comps[3] == ".Trashes"
        && (comps[4] == uid || unsafe { libc::geteuid() } == 0);
    // /Applications/Install macOS <name>.app
    let macos_installer = comps.len() == 3
        && comps[1] == "Applications"
        && comps[2].starts_with("Install macOS")
        && comps[2].ends_with(".app");
    // rotated / archived system logs only
    let system_log = path.starts_with("/private/var/log")
        && path
            .extension()
            .is_some_and(|e| e == "gz" || e == "asl" || e == "bz2");
    // An uninstalled app's system-level files: direct children only, never Apple's.
    let system_support = comps.len() == 4
        && comps[1] == "Library"
        && matches!(
            comps[2].as_str(),
            "Application Support" | "LaunchAgents" | "LaunchDaemons" | "PrivilegedHelperTools" | "Preferences"
        )
        && !comps[3].starts_with("com.apple")
        && !comps[3].starts_with("Apple");
    let in_root = allowed_roots
        .iter()
        .any(|r| path.starts_with(r) && path != r);
    if !(in_root || external_trash || macos_installer || system_log || app_bundle || system_support) {
        return Err("outside allowed locations".into());
    }
    if let Some(w) = whitelist
        .iter()
        .find(|w| path.starts_with(w) || w.starts_with(path))
    {
        return Err(format!("whitelisted ({})", w.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety_gate() {
        let home = Path::new("/Users/me");
        let ok = |p: &str| is_safe_to_delete(Path::new(p), home, &[]).is_ok();
        assert!(ok("/Users/me/Library/Caches/com.foo"));
        assert!(ok("/Users/me/code/app/node_modules"));
        assert!(!ok("/Users/me"));
        assert!(!ok("/Users/me/Library/Caches"));
        assert!(!ok("/Users/me/Documents"));
        assert!(!ok("/Users/me/.ssh/id_rsa"));
        assert!(!ok("/System/Library/Caches/x"));
        assert!(!ok("/Users/me/code/../Documents/x"));
        assert!(ok("/Applications/Install macOS Sequoia.app"));
        assert!(!ok("/Applications/Safari.app"));
        assert!(!ok("/Library/LaunchDaemons/com.apple.foo.plist"));
        assert!(ok("/Library/LaunchDaemons/com.vendor.helper.plist"));
        assert!(!ok("/Library/LaunchDaemons"));
        assert!(!ok("/Library/Application Support/Apple"));
        assert!(!ok("/Library/Frameworks/Foo.framework"));
        assert!(ok("/private/var/log/system.log.0.gz"));
        assert!(!ok("/private/var/log/system.log"));
        let uid = unsafe { libc::getuid() };
        assert!(ok(&format!("/Volumes/USB/.Trashes/{uid}/old.mov")));
        assert!(!ok("/Volumes/USB/Movies/old.mov"));
        let wl = vec![PathBuf::from("/Users/me/Library/Caches/keep")];
        assert!(
            is_safe_to_delete(Path::new("/Users/me/Library/Caches/keep/a"), home, &wl).is_err()
        );
    }

    #[test]
    fn trash_renames_on_collision() {
        let home = std::env::temp_dir().join(format!("deepclean-trash-{}", std::process::id()));
        fs::create_dir_all(home.join(".Trash")).unwrap();
        for _ in 0..2 {
            fs::write(home.join("a.txt"), b"x").unwrap();
            move_to_trash(&home.join("a.txt"), &home).unwrap();
        }
        assert!(home.join(".Trash/a.txt").exists());
        assert!(home.join(".Trash/a 2.txt").exists());
        // same-named folders moved in parallel never collide or overwrite
        let dirs: Vec<PathBuf> = (0..8).map(|i| home.join(format!("src{i}/com.vendor.same"))).collect();
        for d in &dirs {
            fs::create_dir_all(d).unwrap();
            fs::write(d.join("f"), b"y").unwrap();
        }
        let failures: usize = dirs.par_iter().filter(|d| move_to_trash(d, &home).is_err()).count();
        assert_eq!(failures, 0);
        let n = fs::read_dir(home.join(".Trash")).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("com.vendor.same")).count();
        assert_eq!(n, 8);
        assert!(home.join(".Trash/com.vendor.same 2").exists());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn delete_and_measure() {
        let root = std::env::temp_dir().join(format!("deepclean-test-{}", std::process::id()));
        let deep = root.join("a/b/c");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("f"), vec![0u8; 10_000]).unwrap();
        fs::write(root.join("g"), b"hi").unwrap();
        // read-only dir, like the Go module cache
        fs::set_permissions(root.join("a/b"), fs::Permissions::from_mode(0o555)).unwrap();
        assert!(disk_usage(&root, &InodeSet::default()) >= 10_000);
        assert_eq!(remove_tree(&root), 0);
        assert!(!root.exists());
    }
}
