//! Scanners beyond the cache catalog: Downloads hygiene, large files, and
//! leftovers from apps that are no longer installed.

use crate::fsutil::{FILES_SEEN, age_days};
use crate::model::{Action, Target, group};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

/// True if this process may read TCC-protected data (Full Disk Access).
pub fn has_full_disk_access() -> bool {
    fs::File::open("/Library/Application Support/com.apple.TCC/TCC.db").is_ok()
}

/// Days since a Downloads entry last changed *or arrived* (ctime moves when a
/// file is moved/renamed into the folder).
fn idle_days(md: &fs::Metadata) -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let last = md.mtime().max(md.ctime());
    ((now - last).max(0) / 86_400) as u64
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Archive file name → the folder name it would extract to.
fn archive_stem(name: &str) -> Option<&str> {
    let lower = name.to_lowercase();
    [".tar.gz", ".tar.xz", ".tar.bz2", ".tgz", ".zip", ".rar", ".7z", ".tar"]
        .iter()
        .find(|e| lower.ends_with(*e))
        .map(|e| &name[..name.len() - e.len()])
}

pub fn downloads(home: &Path, include_admin: bool) -> Vec<Target> {
    const INSTALLERS: &[&str] = &["dmg", "pkg", "mpkg", "iso", "xip"];
    let mut out = Vec::new();
    let mut claimed = HashSet::new();

    for dir in ["Downloads", "Desktop"] {
        let base = home.join(dir);
        let Ok(rd) = fs::read_dir(&base) else { continue };
        let entries: Vec<(PathBuf, String, fs::Metadata)> = rd
            .flatten()
            .filter_map(|e| Some((e.path(), e.file_name().to_string_lossy().to_string(), e.metadata().ok()?)))
            .collect();
        let names: HashSet<&str> = entries.iter().map(|(_, n, _)| n.as_str()).collect();

        for (path, name, md) in &entries {
            let cat = format!("~/{dir}");
            if md.is_file() && INSTALLERS.contains(&ext_of(path).as_str()) {
                out.push(
                    Target::new(group::DOWNLOADS, cat, name, Action::Delete { paths: vec![path.clone()] })
                        .note(format!("installer · {} days old", age_days(path))),
                );
                claimed.insert(path.clone());
            } else if md.is_file()
                && let Some(stem) = archive_stem(name)
                && names.contains(stem)
                && base.join(stem).is_dir()
            {
                out.push(
                    Target::new(group::DOWNLOADS, cat, name, Action::Delete { paths: vec![path.clone()] })
                        .note("already extracted next to it"),
                );
                claimed.insert(path.clone());
            }
        }

        // Anything in Downloads idle for 90+ days — opt-in. Big ones are listed
        // individually; the long tail of small files is one item.
        if dir == "Downloads" {
            let mut small = Vec::new();
            for (path, name, md) in &entries {
                if claimed.contains(path) || name.starts_with('.') || idle_days(md) < 90 {
                    continue;
                }
                let big = md.is_dir() || md.blocks() * 512 >= 10_000_000;
                if big {
                    out.push(
                        Target::new(group::DOWNLOADS, "~/Downloads · old", name, Action::Delete { paths: vec![path.clone()] })
                            .on(false)
                            .note(format!("untouched for {} days", idle_days(md))),
                    );
                } else {
                    small.push(path.clone());
                }
            }
            if !small.is_empty() {
                let n = small.len();
                out.push(
                    Target::new(group::DOWNLOADS, "~/Downloads · old", format!("{n} smaller old files"), Action::Delete { paths: small })
                        .on(false)
                        .note("each under 10 MB, untouched for 90+ days"),
                );
            }
        }
    }

    // Leftover "Install macOS …" apps (12+ GB each)
    if include_admin && let Ok(rd) = fs::read_dir("/Applications") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("Install macOS") && name.ends_with(".app") {
                out.push(
                    Target::new(group::DOWNLOADS, "/Applications", name, Action::Delete { paths: vec![e.path()] })
                        .admin(true)
                        .note("macOS installer, re-download from the App Store"),
                );
            }
        }
    }
    out
}

/// Individual files ≥ `min_bytes` anywhere in the home folder (outside
/// Library and hidden dirs). Always opt-in: these are the user's own files.
pub fn large_files(home: &Path, min_bytes: u64, exclude: &HashSet<PathBuf>) -> Vec<Target> {
    fn walk(dir: &Path, depth: usize, min: u64, out: &mut Vec<(PathBuf, u64)>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        let mut subdirs = Vec::new();
        let mut n = 0;
        for e in rd.flatten() {
            n += 1;
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                let skip = (depth == 0 && matches!(&*name, "Library" | "Applications"))
                    || [".app", ".photoslibrary", ".musiclibrary", ".tvlibrary", ".fcpbundle", ".logicx", ".bundle"]
                        .iter()
                        .any(|x| name.ends_with(x))
                    || matches!(&*name, "node_modules" | "target" | ".git");
                if !skip {
                    subdirs.push(e.path());
                }
            } else if ft.is_file()
                && let Ok(md) = e.metadata()
                && md.blocks() * 512 >= min
            {
                out.push((e.path(), md.blocks() * 512));
            }
        }
        FILES_SEEN.fetch_add(n, Ordering::Relaxed);
        let found: Vec<Vec<(PathBuf, u64)>> = subdirs
            .par_iter()
            .map(|d| {
                let mut v = Vec::new();
                walk(d, depth + 1, min, &mut v);
                v
            })
            .collect();
        out.extend(found.into_iter().flatten());
    }

    let mut found = Vec::new();
    walk(home, 0, min_bytes, &mut found);
    found.sort_by_key(|f| std::cmp::Reverse(f.1));
    let dupes = duplicates(&found);
    found
        .into_iter()
        .filter(|(p, _)| !exclude.contains(p))
        .take(200)
        .map(|(p, size)| {
            let parent = crate::fsutil::tilde(p.parent().unwrap_or(home), home);
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            let note = match dupes.get(&p) {
                Some(orig) => format!("duplicate of {}", crate::fsutil::tilde(orig, home)),
                None => format!("{} days old", age_days(&p)),
            };
            let mut t = Target::new(group::LARGE, parent, name, Action::Delete { paths: vec![p.clone()] })
                .on(false)
                .note(note);
            t.size = Some(size);
            t
        })
        .collect()
}

/// Byte-identical copies among `files` (same size, then same content hash).
/// Maps each extra copy to the copy that is kept (the oldest path).
fn duplicates(files: &[(PathBuf, u64)]) -> std::collections::HashMap<PathBuf, PathBuf> {
    use std::collections::HashMap;
    use std::hash::Hasher;
    let mut by_size: HashMap<u64, Vec<&PathBuf>> = HashMap::new();
    for (p, _) in files {
        if let Ok(md) = fs::metadata(p) {
            by_size.entry(md.len()).or_default().push(p);
        }
    }
    let hash = |p: &PathBuf| -> Option<u64> {
        let mut f = fs::File::open(p).ok()?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = std::io::Read::read(&mut f, &mut buf).ok()?;
            if n == 0 {
                break;
            }
            h.write(&buf[..n]);
        }
        Some(h.finish())
    };
    let mut out = HashMap::new();
    for group in by_size.into_values().filter(|g| g.len() > 1) {
        let hashed: Vec<(&PathBuf, Option<u64>)> = group.par_iter().map(|p| (*p, hash(p))).collect();
        let mut first: HashMap<u64, &PathBuf> = HashMap::new();
        let mut sorted = hashed;
        sorted.sort_by_key(|(p, _)| fs::metadata(p).and_then(|m| m.modified()).ok());
        for (p, h) in sorted {
            let Some(h) = h else { continue };
            match first.get(&h) {
                Some(orig) => {
                    out.insert(p.clone(), (*orig).clone());
                }
                None => {
                    first.insert(h, p);
                }
            }
        }
    }
    out
}

/// Bundle identifiers of every installed app and its embedded helpers.
fn installed_bundle_ids(home: &Path) -> HashSet<String> {
    fn read_id(bundle: &Path) -> Option<String> {
        let v = plist::Value::from_file(bundle.join("Contents/Info.plist")).ok()?;
        v.as_dictionary()?.get("CFBundleIdentifier")?.as_string().map(str::to_string)
    }
    fn find_apps(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "app") {
                out.push(p);
            } else if depth < 3 && e.file_type().is_ok_and(|t| t.is_dir()) {
                find_apps(&p, depth + 1, out);
            }
        }
    }
    let mut apps = Vec::new();
    for root in [
        PathBuf::from("/Applications"),
        home.join("Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Library/CoreServices"),
        PathBuf::from("/Library/Application Support"),
    ] {
        find_apps(&root, 0, &mut apps);
    }
    apps.par_iter()
        .flat_map_iter(|app| {
            let mut ids: Vec<String> = read_id(app).into_iter().collect();
            for sub in ["PlugIns", "Library/LoginItems", "Helpers", "XPCServices", "Library/SystemExtensions",
                        "Library/LaunchServices", "Frameworks"] {
                if let Ok(rd) = fs::read_dir(app.join("Contents").join(sub)) {
                    ids.extend(rd.flatten().filter_map(|e| read_id(&e.path())));
                }
            }
            ids
        })
        .collect()
}

fn prefix3(id: &str) -> String {
    id.split('.').take(3).collect::<Vec<_>>().join(".").to_lowercase()
}

/// Data folders named after bundle ids whose app is gone. Opt-in, and only
/// when nothing installed shares the id's first three components.
pub fn leftovers(home: &Path) -> Vec<Target> {
    let installed = installed_bundle_ids(home);
    if installed.len() < 10 {
        return Vec::new(); // couldn't enumerate apps — don't guess
    }
    let known: HashSet<String> = installed.iter().map(|i| prefix3(i)).collect();
    let installed_lower: Vec<String> = installed.iter().map(|i| i.to_lowercase()).collect();

    let is_orphan = |id: &str| {
        let l = id.to_lowercase();
        l.matches('.').count() >= 2
            && !l.contains(' ')
            && !l.starts_with("com.apple.")
            && !l.starts_with("group.")
            && !known.contains(&prefix3(&l))
            && !installed_lower.iter().any(|i| l.starts_with(i.as_str()) || i.starts_with(l.as_str()))
    };

    let mut by_id: std::collections::BTreeMap<String, Vec<PathBuf>> = Default::default();
    for dir in ["Library/Containers", "Library/Application Support", "Library/HTTPStorages", "Library/WebKit"] {
        let Ok(rd) = fs::read_dir(home.join(dir)) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let path = e.path();
            if is_orphan(&name) && path.is_dir() && age_days(&path) >= 30 {
                by_id.entry(name).or_default().push(path);
            }
        }
    }
    by_id
        .into_iter()
        .map(|(id, paths)| {
            let age = paths.iter().map(|p| age_days(p)).min().unwrap_or(0);
            Target::new(group::LEFTOVERS, "App no longer installed", id, Action::Delete { paths })
                .on(false)
                .note(format!("untouched for {age} days"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_stems() {
        assert_eq!(archive_stem("foo-main.zip"), Some("foo-main"));
        assert_eq!(archive_stem("x.tar.gz"), Some("x"));
        assert_eq!(archive_stem("notes.txt"), None);
    }
}
