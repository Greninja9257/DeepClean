mod analyze;
mod api;
mod apps;
mod catalog;
mod clean;
mod commands;
mod config;
mod extra;
mod fsutil;
mod model;
mod purge;
mod scan;
mod settings;
mod ui;

use clap::{Parser, Subcommand};
use console::style;
use dialoguer::{Select, theme::ColorfulTheme};
use model::Target;
use std::path::PathBuf;
use std::time::Instant;
use ui::Layout;

/// DeepClean: a fast, deep macOS disk cleaner.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Don't write ~/Library/Logs/deepclean/operations.log
    #[arg(long, global = true)]
    no_log: bool,
    /// Home folder to clean when running as root (used by the app's admin step)
    #[arg(long, global = true, hide = true)]
    home: Option<PathBuf>,
}

#[derive(clap::Args, Clone, Copy)]
struct Common {
    /// Show what would be deleted without touching anything
    #[arg(short = 'n', long)]
    dry_run: bool,
    /// Skip the picker and clean everything selected by default
    #[arg(short, long)]
    yes: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Clean caches, logs, trash, browser & developer-tool junk
    Clean {
        #[command(flatten)]
        common: Common,
        /// Also scan sandboxed app containers (macOS may ask for permission)
        #[arg(long)]
        containers: bool,
    },
    /// Remove rebuildable project artifacts (node_modules, target, build, .venv, …)
    Purge {
        /// Directories to scan (default: your home folder)
        paths: Vec<PathBuf>,
        #[command(flatten)]
        common: Common,
        /// Skip artifacts modified within this many days
        #[arg(long, default_value_t = 7)]
        days: u64,
        /// Maximum folder depth to search
        #[arg(long, default_value_t = 12)]
        depth: usize,
    },
    /// Installers, already-extracted archives and old files in Downloads / Desktop
    Installers {
        #[command(flatten)]
        common: Common,
    },
    /// Manage paths DeepClean must never delete
    Whitelist {
        #[command(subcommand)]
        action: Option<WhitelistCmd>,
    },
    /// Show recent cleanup operations
    History {
        #[arg(short = 'n', default_value_t = 30)]
        lines: usize,
    },
    /// Uninstall an app and everything it left behind (moves it all to the Trash)
    Uninstall {
        /// App name or path; omit to list installed apps by size
        app: Option<String>,
        #[command(flatten)]
        common: Common,
    },
    /// Show what's taking up space inside a folder
    Analyze {
        /// Folder to analyze (default: your home folder)
        path: Option<PathBuf>,
        /// How many entries to show
        #[arg(short = 'n', long, default_value_t = 25)]
        top: usize,
    },
    /// Run maintenance tasks (flush DNS, rebuild Launch Services, …)
    Optimize {
        #[command(flatten)]
        common: Common,
    },
    /// Scan everything and stream JSON (for the app)
    #[command(hide = true)]
    ScanJson,
    #[command(hide = true)]
    AppsJson,
    #[command(hide = true)]
    UninstallScanJson { app: PathBuf },
    #[command(hide = true)]
    AnalyzeJson { path: PathBuf },
    #[command(hide = true)]
    OptimizeJson,
    /// Clean the items in a JSON request (for the app)
    #[command(hide = true)]
    CleanJson {
        #[arg(long)]
        input: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum WhitelistCmd {
    List,
    Add { path: String },
    Remove { path: String },
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn home_dir(explicit: Option<PathBuf>) -> PathBuf {
    if is_root() {
        // Under sudo / the app's admin prompt, still clean the user's home.
        let candidate = explicit.or_else(|| std::env::var("SUDO_USER").ok().map(|u| PathBuf::from("/Users").join(u)));
        if let Some(p) = candidate.filter(|p| p.starts_with("/Users") && p.components().count() == 3 && p.is_dir()) {
            return p;
        }
    }
    std::env::var_os("HOME").map(PathBuf::from).expect("HOME is not set")
}

fn banner() {
    println!(
        "\n  {} {}\n",
        style("DeepClean").cyan().bold(),
        style(format!("v{}, deep macOS cleaner", env!("CARGO_PKG_VERSION"))).dim()
    );
}

struct App {
    home: PathBuf,
    whitelist: Vec<PathBuf>,
    log: config::OpLog,
}

impl App {
    fn finish(&mut self, targets: Vec<Target>, layout: Layout, common: Common, started: Instant) {
        if targets.is_empty() {
            println!("  {} Nothing to clean.", style("✔").green());
            return;
        }
        println!("  {}", style(format!("Scanned in {:.2}s", started.elapsed().as_secs_f64())).dim());
        ui::print_list(&targets, layout);

        let chosen: Vec<usize> = if common.yes || common.dry_run {
            (0..targets.len()).filter(|&i| targets[i].default_on).collect()
        } else {
            match ui::pick(&targets, layout) {
                Some(c) => c,
                None => {
                    println!("  Cancelled.");
                    return;
                }
            }
        };
        let chosen: Vec<&Target> = chosen.iter().map(|&i| &targets[i]).collect();
        if chosen.is_empty() {
            println!("  Nothing selected.");
            return;
        }
        if common.dry_run {
            ui::dry_run(&chosen, &self.home);
            return;
        }
        if !common.yes {
            let bytes: u64 = chosen.iter().map(|t| t.bytes()).sum();
            let prompt = format!("Permanently delete {} item(s), about {}?", chosen.len(), fsutil::human(bytes));
            if !ui::confirm(&prompt) {
                println!("  Cancelled.");
                return;
            }
        }
        println!();
        ui::execute(&chosen, &self.home, &self.whitelist, &mut self.log);
        println!();
    }

    fn clean(&mut self, common: Common, containers: bool) {
        let started = Instant::now();
        let opts = catalog::ScanOptions {
            admin: is_root(),
            containers: containers || extra::has_full_disk_access(),
        };
        let mut targets = catalog::build_targets(&self.home, &opts);
        targets.extend(commands::discover(&self.home, is_root()));
        ui::measure(&mut targets, &self.whitelist, "Scanning caches & junk…");
        // keep each category together (commands were appended last)
        let mut order: Vec<String> = Vec::new();
        for t in &targets {
            if !order.contains(&t.category) {
                order.push(t.category.clone());
            }
        }
        targets.sort_by_key(|t| order.iter().position(|c| *c == t.category));
        if !is_root() {
            println!("  {}", style("Tip: run with sudo to include system-wide caches.").dim());
        }
        self.finish(targets, Layout::Catalog, common, started);
    }

    fn purge(&mut self, paths: Vec<PathBuf>, common: Common, days: u64, depth: usize) {
        let started = Instant::now();
        let roots = if paths.is_empty() { vec![self.home.clone()] } else { paths };
        let opts = purge::PurgeOptions { roots, min_age_days: days, max_depth: depth };

        let scan = ui::with_counter("Hunting for project artifacts…", || purge::scan(&self.home, &opts));
        let mut targets = scan.targets;
        ui::measure(&mut targets, &self.whitelist, "Measuring…");
        targets.sort_by_key(|t| std::cmp::Reverse(t.bytes()));
        if scan.skipped_recent + scan.skipped_protected > 0 {
            println!(
                "  {}",
                style(format!(
                    "Kept {} active (<{days}d) and {} protected (git-tracked / key files) artifact(s).",
                    scan.skipped_recent, scan.skipped_protected
                ))
                .dim()
            );
        }
        self.finish(targets, Layout::Items, common, started);
    }

    fn uninstall(&mut self, query: Option<String>, common: Common) {
        let Some(q) = query else {
            let mut apps = ui::with_counter("Measuring installed apps…", || apps::list(&self.home));
            apps.sort_by_key(|a| std::cmp::Reverse(a.size));
            for a in &apps {
                let lock = if a.admin { " 🔒" } else { "" };
                println!(
                    "  {}  {}{}",
                    style(format!("{:>9}", fsutil::human(a.size))).yellow().bold(),
                    a.name,
                    style(format!("  {}{lock}", a.version)).dim()
                );
            }
            println!("\n  {}", style("Run `deepclean uninstall <name>` to remove one.").dim());
            return;
        };
        let started = Instant::now();
        let Some(found) = apps::find(&self.home, &q) else {
            println!("  No installed app matches “{q}”.");
            return;
        };
        let mut targets = apps::related(&self.home, &found.path);
        targets.retain(|t| !t.paths().iter().any(|p| self.whitelist.iter().any(|w| p.starts_with(w))));
        if !is_root() && targets.iter().any(|t| t.admin) {
            println!("  {}", style("Some files need administrator rights; run with sudo to remove them too.").dim());
            targets.retain(|t| !t.admin);
        }
        println!("  {} {}\n", style("Uninstalling").bold(), style(&found.name).cyan().bold());
        self.finish(targets, Layout::Catalog, common, started);
        println!("  {}", style("Everything was moved to the Trash. Empty it to free the space.").dim());
    }

    fn analyze(&mut self, path: Option<PathBuf>, top: usize) {
        let dir = path.unwrap_or_else(|| self.home.clone());
        let a = ui::with_counter("Measuring…", || analyze::analyze(&dir));
        println!("  {}  {}\n", style(fsutil::tilde(&a.path, &self.home)).bold(), style(fsutil::human(a.total)).yellow().bold());
        let max = a.entries.first().map(|e| e.size).unwrap_or(1).max(1);
        for e in a.entries.iter().take(top) {
            let bar = "█".repeat(((e.size as f64 / max as f64) * 24.0).round() as usize);
            let name = if e.is_dir { format!("{}/", e.name) } else { e.name.clone() };
            println!("  {:>9}  {:<24}  {}", fsutil::human(e.size), style(bar).cyan(), name);
        }
        if a.entries.len() > top {
            println!("  {}", style(format!("… and {} more", a.entries.len() - top)).dim());
        }
    }

    fn optimize(&mut self, common: Common) {
        let started = Instant::now();
        let mut tasks = commands::optimize_tasks();
        if !is_root() {
            for t in tasks.iter_mut().filter(|t| t.admin) {
                t.default_on = false;
                t.note = format!("{} (needs sudo)", t.note);
            }
        }
        self.finish(tasks, Layout::Catalog, common, started);
    }

    fn installers(&mut self, common: Common) {
        let started = Instant::now();
        let s = settings::load(&self.home);
        let mut targets: Vec<Target> = extra::downloads(&self.home, is_root(), s.old_download_days)
            .into_iter()
            .map(|t| t.trash(s.trash_personal))
            .collect();
        ui::measure(&mut targets, &self.whitelist, "Looking through Downloads…");
        targets.sort_by_key(|t| (!t.default_on, std::cmp::Reverse(t.bytes())));
        self.finish(targets, Layout::Items, common, started);
    }
}

fn main() {
    let cli = Cli::parse();
    let home = home_dir(cli.home);
    let mut app = App {
        whitelist: config::load_whitelist(&home),
        log: config::OpLog::open(&home, !cli.no_log),
        home,
    };
    let none = Common { dry_run: false, yes: false };

    match cli.cmd {
        Some(Cmd::Clean { common, containers }) => {
            banner();
            app.clean(common, containers)
        }
        Some(Cmd::Purge { paths, common, days, depth }) => {
            banner();
            app.purge(paths, common, days, depth)
        }
        Some(Cmd::Installers { common }) => {
            banner();
            app.installers(common)
        }
        Some(Cmd::Uninstall { app: name, common }) => {
            banner();
            app.uninstall(name, common)
        }
        Some(Cmd::Analyze { path, top }) => {
            banner();
            app.analyze(path, top)
        }
        Some(Cmd::Optimize { common }) => {
            banner();
            app.optimize(common)
        }
        Some(Cmd::ScanJson) => api::scan_json(&app.home, &app.whitelist),
        Some(Cmd::AppsJson) => api::apps_json(&app.home),
        Some(Cmd::UninstallScanJson { app: path }) => api::uninstall_scan_json(&app.home, &app.whitelist, &path),
        Some(Cmd::AnalyzeJson { path }) => api::analyze_json(&path),
        Some(Cmd::OptimizeJson) => api::optimize_json(&app.home),
        Some(Cmd::CleanJson { input }) => api::clean_json(&app.home, &app.whitelist, input.as_deref(), &mut app.log),
        Some(Cmd::Whitelist { action }) => match action.unwrap_or(WhitelistCmd::List) {
            WhitelistCmd::List => {
                println!("{}", style(config::whitelist_file(&app.home).display()).dim());
                for p in &app.whitelist {
                    println!("  {}", p.display());
                }
            }
            WhitelistCmd::Add { path } => match config::whitelist_add(&app.home, &path) {
                Ok(p) => println!("Protected {}", p.display()),
                Err(e) => eprintln!("error: {e}"),
            },
            WhitelistCmd::Remove { path } => match config::whitelist_remove(&app.home, &path) {
                Ok(true) => println!("Removed {path}"),
                Ok(false) => println!("{path} was not in the whitelist"),
                Err(e) => eprintln!("error: {e}"),
            },
        },
        Some(Cmd::History { lines }) => {
            let text = std::fs::read_to_string(config::log_dir(&app.home).join("operations.log")).unwrap_or_default();
            let all: Vec<&str> = text.lines().collect();
            for l in &all[all.len().saturating_sub(lines)..] {
                println!("{l}");
            }
        }
        None => {
            banner();
            let items = [
                "Clean       caches, logs, trash, browser & dev-tool junk",
                "Purge       project artifacts (node_modules, target, build, .venv…)",
                "Downloads   installers, extracted archives, old files",
                "Quit",
            ];
            let choice = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("What would you like to do?")
                .items(items)
                .default(0)
                .interact_opt()
                .ok()
                .flatten();
            match choice {
                Some(0) => app.clean(none, false),
                Some(1) => app.purge(vec![], none, 7, 12),
                Some(2) => app.installers(none),
                _ => {}
            }
        }
    }
}
