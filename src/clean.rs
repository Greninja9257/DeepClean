//! Executes a cleaning plan: parallel deletions behind the safety gate, then
//! whitelisted commands. Shared by the CLI and the JSON API.

use crate::commands;
use crate::config::OpLog;
use crate::fsutil::{self, InodeSet};
use crate::model::{Action, Target};
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Bytes of selected targets finished so far (for progress displays).
pub static BYTES_DONE: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize)]
pub struct Outcome {
    pub id: String,
    pub name: String,
    pub freed: u64,
    /// Bytes moved to the Trash (not freed until the Trash is emptied).
    pub trashed: u64,
    pub failures: u64,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct Report {
    pub outcomes: Vec<Outcome>,
    pub refused: Vec<String>,
    pub freed: u64,
    pub trashed: u64,
    pub free_before: Option<u64>,
    pub free_after: Option<u64>,
}

pub fn run(targets: &[&Target], home: &Path, whitelist: &[PathBuf], log: &mut OpLog) -> Report {
    BYTES_DONE.store(0, Ordering::Relaxed);
    fsutil::FILES_DELETED.store(0, Ordering::Relaxed);
    let free_before = fsutil::free_space(home);

    // Safety gate first, before anything is touched.
    let mut refused = Vec::new();
    let mut jobs: Vec<(&Target, Vec<&PathBuf>)> = Vec::new();
    for t in targets {
        if let Action::Delete { paths } = &t.action {
            let ok = paths
                .iter()
                .filter(|p| match fsutil::is_safe_to_delete(p, home, whitelist) {
                    Ok(()) => true,
                    Err(why) => {
                        refused.push(format!("{} ({why})", fsutil::tilde(p, home)));
                        false
                    }
                })
                .collect();
            jobs.push((t, ok));
        }
    }

    // Deletions: parallel across targets and within each tree.
    let mut outcomes: Vec<Outcome> = jobs
        .par_iter()
        .map(|(t, paths)| {
            if t.trash {
                let mut moved = 0;
                let mut failures = 0;
                let mut error = None;
                for p in paths {
                    match fsutil::move_to_trash(p, home) {
                        Ok(()) => moved += 1,
                        Err(e) => {
                            failures += 1;
                            error = Some(e.to_string());
                        }
                    }
                }
                BYTES_DONE.fetch_add(t.bytes(), Ordering::Relaxed);
                let share = if paths.is_empty() { 0 } else { t.bytes() * moved / paths.len() as u64 };
                return Outcome { id: t.id.clone(), name: t.name.clone(), freed: 0, trashed: share, failures, error };
            }
            let failures: u64 = paths.iter().map(|p| fsutil::remove_tree(p)).sum();
            // Partial failure: whatever is still on disk wasn't freed.
            let left: u64 = if failures > 0 {
                let seen = InodeSet::default();
                paths.iter().map(|p| fsutil::disk_usage(p, &seen)).sum()
            } else {
                0
            };
            BYTES_DONE.fetch_add(t.bytes(), Ordering::Relaxed);
            Outcome {
                id: t.id.clone(),
                name: t.name.clone(),
                freed: t.bytes().saturating_sub(left),
                trashed: 0,
                failures,
                error: None,
            }
        })
        .collect();
    for ((t, paths), o) in jobs.iter().zip(&outcomes) {
        for p in paths {
            let status = match (t.trash, o.failures) {
                (true, 0) => "trashed",
                (false, 0) => "deleted",
                _ => "partial",
            };
            log.record(status, 0, &p.display().to_string());
        }
        log.record(if t.trash { "trashed-target" } else { "target" }, o.freed + o.trashed, &t.name);
    }

    // Commands: sequential, measured by the change in free space.
    for t in targets {
        let Action::Command { command } = &t.action else { continue };
        let before = fsutil::free_space(home).unwrap_or(0);
        let error = match commands::resolve(command) {
            None => Some("unknown command".to_string()),
            Some(cmds) => cmds.iter().find_map(|c| match Command::new(&c[0]).args(&c[1..]).output() {
                Ok(o) if o.status.success() => None,
                Ok(o) => Some(
                    String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("failed").trim().to_string(),
                ),
                Err(e) => Some(e.to_string()),
            }),
        };
        let freed = fsutil::free_space(home).unwrap_or(0).saturating_sub(before);
        BYTES_DONE.fetch_add(t.bytes(), Ordering::Relaxed);
        log.record("command", freed, command);
        outcomes.push(Outcome { id: t.id.clone(), name: t.name.clone(), freed, trashed: 0, failures: 0, error });
    }

    let freed = outcomes.iter().map(|o| o.freed).sum();
    let trashed = outcomes.iter().map(|o| o.trashed).sum();
    Report { outcomes, refused, freed, trashed, free_before, free_after: fsutil::free_space(home) }
}
