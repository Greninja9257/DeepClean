//! Terminal front end: spinners, listing, the picker and the result report.

use crate::clean::{self, BYTES_DONE};
use crate::config::OpLog;
use crate::fsutil::{BYTES_SEEN, FILES_DELETED, FILES_SEEN, human};
use crate::model::{Action, Target};
use console::{Term, style, truncate_str};
use dialoguer::{Confirm, MultiSelect, theme::ColorfulTheme};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
pub enum Layout {
    /// Category + name columns (cache catalog).
    Catalog,
    /// One line per path: size, kind, location (project artifacts, installers).
    Items,
}

pub fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::with_template("{spinner:.cyan} {msg}").unwrap());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb.set_message(msg.to_string());
    pb
}

/// Runs `work` while a spinner shows live file / byte counters.
pub fn with_counter<T: Send>(label: &str, work: impl FnOnce() -> T + Send) -> T {
    FILES_SEEN.store(0, Ordering::Relaxed);
    BYTES_SEEN.store(0, Ordering::Relaxed);
    let pb = spinner(label);
    let done = AtomicBool::new(false);
    let out = std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                let f = FILES_SEEN.load(Ordering::Relaxed);
                let b = BYTES_SEEN.load(Ordering::Relaxed);
                let extra = if b > 0 { format!(", {}", human(b)) } else { String::new() };
                pb.set_message(format!("{label}  {}", style(format!("{f} files{extra}")).dim()));
                std::thread::sleep(Duration::from_millis(60));
            }
        });
        let out = work();
        done.store(true, Ordering::Relaxed);
        out
    });
    pb.finish_and_clear();
    out
}

pub fn measure(targets: &mut Vec<Target>, whitelist: &[PathBuf], label: &str) {
    with_counter(label, || crate::scan::measure(targets, whitelist));
}

fn size_cell(t: &Target) -> String {
    t.size.map(human).unwrap_or_else(|| "—".into())
}

fn plain_label(t: &Target, layout: Layout, width: usize) -> String {
    let lock = if t.admin { " 🔒" } else { "" };
    let line = match layout {
        Layout::Catalog => format!("{:<11} {:<42} {:>9}  {}{lock}", t.category, t.name, size_cell(t), t.note),
        Layout::Items => format!("{:>9}  {:<14} {}  {}{lock}", size_cell(t), t.name, t.category, t.note),
    };
    truncate_str(&line, width.saturating_sub(6), "…").to_string()
}

pub fn print_list(targets: &[Target], layout: Layout) {
    let width = Term::stdout().size().1 as usize;
    let mut last_cat = "";
    for t in targets {
        if layout == Layout::Catalog && t.category != last_cat {
            println!("\n  {}", style(&t.category).bold().underlined());
            last_cat = &t.category;
        }
        let mark = if t.default_on { style("●").green() } else { style("○").dim() };
        let size = style(format!("{:>9}", size_cell(t))).yellow().bold();
        let body = match layout {
            Layout::Catalog => format!("{:<42}", t.name),
            Layout::Items => format!("{:<14} {}", t.name, t.category),
        };
        let line = format!("  {mark} {size}  {body}  {}", style(&t.note).dim());
        println!("{}", truncate_str(&line, width.saturating_sub(1), "…"));
    }
    let on: u64 = targets.iter().filter(|t| t.default_on).map(Target::bytes).sum();
    let all: u64 = targets.iter().map(Target::bytes).sum();
    println!(
        "\n  {} recommended ({} selected by default), {} found in total\n",
        style(human(on)).green().bold(),
        style("●").green(),
        style(human(all)).bold()
    );
}

/// Interactive checkbox picker. Returns None if the user backs out.
pub fn pick(targets: &[Target], layout: Layout) -> Option<Vec<usize>> {
    let (rows, cols) = Term::stdout().size();
    let labels: Vec<String> = targets.iter().map(|t| plain_label(t, layout, cols as usize)).collect();
    let defaults: Vec<bool> = targets.iter().map(|t| t.default_on).collect();
    MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose what to clean  (space = toggle, a = all, enter = confirm, esc = cancel)")
        .items(&labels)
        .defaults(&defaults)
        .max_length((rows as usize).saturating_sub(6).max(5))
        .interact_opt()
        .ok()
        .flatten()
}

pub fn confirm(prompt: &str) -> bool {
    Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .default(false)
        .interact()
        .unwrap_or(false)
}

pub fn dry_run(chosen: &[&Target], home: &Path) {
    println!("  {}: nothing will be deleted.\n", style("DRY RUN").yellow().bold());
    for t in chosen {
        match &t.action {
            Action::Delete { paths } => {
                println!("  {} {}  {}", style("would delete").dim(), style(size_cell(t)).yellow(), t.name);
                for p in paths {
                    println!("      {}", style(crate::fsutil::tilde(p, home)).dim());
                }
            }
            Action::Command { command } => {
                for c in crate::commands::resolve(command).unwrap_or_default() {
                    println!("  {} {}", style("would run").dim(), c.join(" "));
                }
            }
        }
    }
    let total: u64 = chosen.iter().map(|t| t.bytes()).sum();
    println!("\n  Would free about {}.", style(human(total)).green().bold());
}

/// Clean with a live progress bar, then print the report.
pub fn execute(chosen: &[&Target], home: &Path, whitelist: &[PathBuf], log: &mut OpLog) {
    let total: u64 = chosen.iter().map(|t| t.bytes()).sum();
    let pb = ProgressBar::new(total.max(1));
    pb.set_style(
        ProgressStyle::with_template("  {spinner:.cyan} [{bar:32.cyan/blue}] {msg}")
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏ "),
    );
    pb.enable_steady_tick(Duration::from_millis(80));
    let done = AtomicBool::new(false);
    let report = std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                pb.set_position(BYTES_DONE.load(Ordering::Relaxed));
                pb.set_message(format!("{} files removed", FILES_DELETED.load(Ordering::Relaxed)));
                std::thread::sleep(Duration::from_millis(60));
            }
        });
        let r = clean::run(chosen, home, whitelist, log);
        done.store(true, Ordering::Relaxed);
        r
    });
    pb.finish_and_clear();

    for o in &report.outcomes {
        let (mark, extra) = match (&o.error, o.failures) {
            (Some(e), _) => (style("✘").red(), style(format!("  {e}")).dim().to_string()),
            (None, 0) => (style("✔").green(), String::new()),
            (None, n) => (style("◐").yellow(), style(format!("  ({n} items in use or permission-denied)")).dim().to_string()),
        };
        println!("  {mark} {:>9}  {}{extra}", human(o.freed), o.name);
    }
    if !report.refused.is_empty() {
        println!("\n  {} skipped by safety rules:", style(report.refused.len()).yellow());
        for r in report.refused.iter().take(10) {
            println!("    {}", style(r).dim());
        }
        if report.refused.len() > 10 {
            println!("    … and {} more", report.refused.len() - 10);
        }
    }
    println!("\n  {} Freed {}", style("✨").bold(), style(human(report.freed)).green().bold());
    if let (Some(b), Some(a)) = (report.free_before, report.free_after) {
        println!("  Disk free: {} → {}", human(b), style(human(a)).bold());
        if a.saturating_sub(b) * 2 < report.freed {
            println!(
                "  {}",
                style("Some space may stay held by APFS/Time Machine local snapshots until macOS purges them.").dim()
            );
        }
    }
}
