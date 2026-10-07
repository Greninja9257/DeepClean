//! Line-delimited JSON protocol used by the macOS app.
//!
//! `scan-json`  → {"type":"progress",…}* {"type":"phase",…}* {"type":"result",…}
//! `clean-json` ← {"items":[…]} on stdin or --input
//!              → {"type":"progress",…}* {"type":"done",…}

use crate::clean::{self, BYTES_DONE};
use crate::config::OpLog;
use crate::fsutil::{self, BYTES_SEEN, FILES_DELETED, FILES_SEEN};
use crate::model::{Action, Target, group};
use crate::scan;
use serde::Deserialize;
use serde_json::json;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn emit(v: serde_json::Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

/// Runs `work` while emitting a progress event every 100 ms.
fn with_ticker<T: Send>(event: impl Fn() -> serde_json::Value + Sync, work: impl FnOnce() -> T + Send) -> T {
    let done = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                emit(event());
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let out = work();
        done.store(true, Ordering::Relaxed);
        out
    })
}

pub fn scan_json(home: &Path, whitelist: &[PathBuf]) {
    let started = Instant::now();
    FILES_SEEN.store(0, Ordering::Relaxed);
    BYTES_SEEN.store(0, Ordering::Relaxed);
    let full = with_ticker(
        || {
            json!({"type": "progress",
                   "files": FILES_SEEN.load(Ordering::Relaxed),
                   "bytes": BYTES_SEEN.load(Ordering::Relaxed)})
        },
        || scan::full(home, whitelist, &crate::settings::load(home), &|phase| emit(json!({"type": "phase", "name": phase}))),
    );
    // JSON can only carry UTF-8 paths; skip the (rare) others rather than fail.
    let items: Vec<Target> = full
        .targets
        .into_iter()
        .filter(|t| t.paths().iter().all(|p| p.to_str().is_some()))
        .collect();
    emit(json!({
        "type": "result",
        "fda": full.fda,
        "home": home,
        "disk": {"total": fsutil::total_space(home), "free": fsutil::free_space(home)},
        "elapsed_ms": started.elapsed().as_millis() as u64,
        "items": items,
    }));
}

#[derive(Deserialize)]
struct Request {
    items: Vec<RequestItem>,
}

#[derive(Deserialize)]
struct RequestItem {
    id: String,
    name: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    paths: Option<Vec<PathBuf>>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    trash: bool,
}

pub fn clean_json(home: &Path, whitelist: &[PathBuf], input: Option<&Path>, log: &mut OpLog) {
    let mut raw = String::new();
    let read = match input {
        Some(p) => std::fs::read_to_string(p).map(|s| raw = s),
        None => std::io::stdin().read_to_string(&mut raw).map(|_| ()),
    };
    let req: Request = match read.map_err(|e| e.to_string()).and_then(|_| serde_json::from_str(&raw).map_err(|e| e.to_string())) {
        Ok(r) => r,
        Err(e) => {
            emit(json!({"type": "error", "message": format!("bad request: {e}")}));
            std::process::exit(2);
        }
    };
    let targets: Vec<Target> = req
        .items
        .into_iter()
        .filter_map(|i| {
            let action = match (i.paths, i.command) {
                (_, Some(command)) => Action::Command { command },
                (Some(paths), None) => Action::Delete { paths },
                (None, None) => return None,
            };
            let mut t = Target::new(group::JUNK, "", i.name, action);
            t.id = i.id;
            t.size = i.size;
            t.trash = i.trash;
            Some(t)
        })
        .collect();
    let refs: Vec<&Target> = targets.iter().collect();
    let total: u64 = refs.iter().map(|t| t.bytes()).sum();
    let report = with_ticker(
        || {
            json!({"type": "progress",
                   "bytes_done": BYTES_DONE.load(Ordering::Relaxed),
                   "bytes_total": total,
                   "files": FILES_DELETED.load(Ordering::Relaxed)})
        },
        || clean::run(&refs, home, whitelist, log),
    );
    let mut v = serde_json::to_value(&report).unwrap_or_default();
    v["type"] = json!("done");
    emit(v);
}

fn emit_items(home: &Path, items: Vec<Target>, started: Instant) {
    let items: Vec<Target> = items.into_iter().filter(|t| t.paths().iter().all(|p| p.to_str().is_some())).collect();
    emit(json!({
        "type": "result",
        "fda": crate::extra::has_full_disk_access(),
        "home": home,
        "disk": {"total": fsutil::total_space(home), "free": fsutil::free_space(home)},
        "elapsed_ms": started.elapsed().as_millis() as u64,
        "items": items,
    }));
}

/// Installed third-party apps with sizes.
pub fn apps_json(home: &Path) {
    let apps = crate::apps::list(home);
    emit(json!({"type": "apps", "items": apps}));
}

/// An app bundle plus its related files, ready to uninstall.
pub fn uninstall_scan_json(home: &Path, whitelist: &[PathBuf], app: &Path) {
    let started = Instant::now();
    let mut items = crate::apps::related(home, app);
    items.retain(|t| !t.paths().iter().any(|p| whitelist.iter().any(|w| p.starts_with(w))));
    emit_items(home, items, started);
}

/// Sizes of everything directly inside `dir`.
pub fn analyze_json(dir: &Path) {
    FILES_SEEN.store(0, Ordering::Relaxed);
    BYTES_SEEN.store(0, Ordering::Relaxed);
    let a = with_ticker(
        || {
            json!({"type": "progress",
                   "files": FILES_SEEN.load(Ordering::Relaxed),
                   "bytes": BYTES_SEEN.load(Ordering::Relaxed)})
        },
        || crate::analyze::analyze(dir),
    );
    let mut v = serde_json::to_value(&a).unwrap_or_default();
    v["type"] = json!("analysis");
    emit(v);
}

/// Maintenance tasks for the Optimize screen.
pub fn optimize_json(home: &Path) {
    emit_items(home, crate::commands::optimize_tasks(), Instant::now());
}
