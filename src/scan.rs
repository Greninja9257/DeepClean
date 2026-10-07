//! Measuring targets and running every scanner at once for the app.

use crate::fsutil::{self, InodeSet};
use crate::model::{Action, Target};
use crate::settings::Settings;
use crate::{catalog, commands, extra, purge};
use crate::model::group;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Drop whitelisted paths, measure every delete target in parallel, then drop
/// empty ones.
pub fn measure(targets: &mut Vec<Target>, whitelist: &[PathBuf]) {
    for t in targets.iter_mut() {
        if let Action::Delete { paths } = &mut t.action {
            paths.retain(|p| !whitelist.iter().any(|w| p.starts_with(w) || w.starts_with(p)));
        }
    }
    let seen = InodeSet::default();
    targets.par_iter_mut().for_each(|t| {
        if let Action::Delete { paths } = &t.action
            && t.size.is_none()
        {
            t.size = Some(paths.par_iter().map(|p| fsutil::disk_usage(p, &seen)).sum());
        }
    });
    targets.retain(|t| matches!(t.action, Action::Command { .. }) || t.bytes() > 0);
}

pub struct Full {
    pub targets: Vec<Target>,
    pub fda: bool,
}

/// Everything DeepClean can find, scanned concurrently. `phase_done` is
/// called (from worker threads) as each scanner finishes.
pub fn full(home: &Path, whitelist: &[PathBuf], settings: &Settings, phase_done: &(dyn Fn(&str) + Sync)) -> Full {
    let fda = extra::has_full_disk_access();
    let measured = |mut v: Vec<Target>, phase: &str| {
        measure(&mut v, whitelist);
        phase_done(phase);
        v
    };

    let ((caches, projects), (downloads, (large, leftovers))) = rayon::join(
        || {
            rayon::join(
                || {
                    let opts = catalog::ScanOptions { admin: true, containers: fda };
                    let mut v = catalog::build_targets(home, &opts);
                    v.extend(commands::discover(home, true));
                    measured(v, "caches")
                },
                || {
                    let opts = purge::PurgeOptions { roots: vec![home.to_path_buf()], min_age_days: settings.project_min_age_days, max_depth: 12 };
                    measured(purge::scan(home, &opts).targets, "projects")
                },
            )
        },
        || {
            rayon::join(
                || measured(extra::downloads(home, true, settings.old_download_days), "downloads"),
                || {
                    rayon::join(
                        || {
                            let v = if settings.scan_large_files {
                                extra::large_files(home, settings.large_file_mb * 1_000_000, &HashSet::new())
                            } else {
                                Vec::new()
                            };
                            phase_done("large");
                            v
                        },
                        || measured(if fda && settings.scan_leftovers { extra::leftovers(home) } else { Vec::new() }, "leftovers"),
                    )
                },
            )
        },
    );

    let mut targets: Vec<Target> = [caches, projects, downloads, leftovers].concat();
    // Large files already covered by another item (e.g. inside a .venv or a
    // Downloads entry) would be counted twice — drop them.
    let claimed: HashSet<&Path> = targets.iter().flat_map(|t| t.paths().iter().map(PathBuf::as_path)).collect();
    let large: Vec<Target> = large
        .into_iter()
        .filter(|t| !t.paths()[0].ancestors().any(|a| claimed.contains(a)))
        .filter(|t| !whitelist.iter().any(|w| t.paths()[0].starts_with(w)))
        .collect();
    targets.extend(large);
    // A password prompt should be opt-in, never part of the default clean.
    for t in targets.iter_mut() {
        if t.admin {
            t.default_on = false;
        }
        if matches!(t.group, group::DOWNLOADS | group::LARGE | group::LEFTOVERS) {
            t.trash = settings.trash_personal;
        }
    }
    Full { targets, fda }
}
