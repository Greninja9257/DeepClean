//! User whitelist (~/.config/deepclean/whitelist) and the operations log
//! (~/Library/Logs/deepclean/operations.log).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn whitelist_file(home: &Path) -> PathBuf {
    home.join(".config/deepclean/whitelist")
}

pub fn log_dir(home: &Path) -> PathBuf {
    home.join("Library/Logs/deepclean")
}

fn expand(line: &str, home: &Path) -> PathBuf {
    match line.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if line == "~" => home.to_path_buf(),
        None => PathBuf::from(line),
    }
}

/// Whitelisted paths, plus DeepClean's own log directory (which lives inside
/// ~/Library/Logs and would otherwise be swept by the logs cleaner).
pub fn load_whitelist(home: &Path) -> Vec<PathBuf> {
    let mut out = vec![log_dir(home)];
    if let Ok(text) = fs::read_to_string(whitelist_file(home)) {
        out.extend(
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| expand(l, home)),
        );
    }
    out
}

pub fn whitelist_add(home: &Path, raw: &str) -> std::io::Result<PathBuf> {
    let p = expand(raw, home);
    let p = fs::canonicalize(&p).unwrap_or(p);
    let file = whitelist_file(home);
    fs::create_dir_all(file.parent().unwrap())?;
    let mut f = OpenOptions::new().create(true).append(true).open(&file)?;
    writeln!(f, "{}", p.display())?;
    Ok(p)
}

pub fn whitelist_remove(home: &Path, raw: &str) -> std::io::Result<bool> {
    let target = expand(raw, home);
    let file = whitelist_file(home);
    let text = fs::read_to_string(&file).unwrap_or_default();
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| expand(l.trim(), home) != target)
        .collect();
    let removed = kept.len() != text.lines().count();
    fs::write(
        &file,
        kept.join("\n") + if kept.is_empty() { "" } else { "\n" },
    )?;
    Ok(removed)
}

pub struct OpLog(Option<fs::File>);

impl OpLog {
    pub fn open(home: &Path, enabled: bool) -> Self {
        if !enabled {
            return OpLog(None);
        }
        let dir = log_dir(home);
        let _ = fs::create_dir_all(&dir);
        let file = dir.join("operations.log");
        let f = OpenOptions::new().create(true).append(true).open(&file).ok();
        // When running as root (admin clean from the app), keep the log owned
        // by the user so later unprivileged runs can still append to it.
        if unsafe { libc::geteuid() } == 0
            && let Ok(md) = fs::metadata(home)
        {
            use std::os::unix::fs::MetadataExt;
            let _ = std::os::unix::fs::chown(&dir, Some(md.uid()), Some(md.gid()));
            let _ = std::os::unix::fs::chown(&file, Some(md.uid()), Some(md.gid()));
        }
        OpLog(f)
    }

    pub fn record(&mut self, status: &str, bytes: u64, what: &str) {
        if let Some(f) = &mut self.0 {
            let ts = humantime::format_rfc3339_seconds(SystemTime::now());
            let _ = writeln!(f, "{ts}\t{status}\t{bytes}\t{what}");
        }
    }
}
