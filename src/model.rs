//! The unit of work every scanner produces and the cleaner consumes.

use serde::Serialize;
use std::path::PathBuf;

/// UI sections, in display order.
pub mod group {
    pub const JUNK: &str = "junk";
    pub const BROWSERS: &str = "browsers";
    pub const APPS: &str = "apps";
    pub const DEV: &str = "dev";
    pub const PROJECTS: &str = "projects";
    pub const DOWNLOADS: &str = "downloads";
    pub const LARGE: &str = "large";
    pub const LEFTOVERS: &str = "leftovers";
    pub const SYSTEM: &str = "system";
    pub const UNINSTALL: &str = "uninstall";
    pub const OPTIMIZE: &str = "optimize";
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Action {
    Delete { paths: Vec<PathBuf> },
    /// A whitelisted external command, referenced by id (see `commands::resolve`).
    Command { command: String },
}

#[derive(Clone, Serialize)]
pub struct Target {
    pub id: String,
    pub group: &'static str,
    /// Sub-heading shown under the name (ecosystem, project folder, …).
    pub category: String,
    pub name: String,
    #[serde(flatten)]
    pub action: Action,
    /// Measured disk usage; None when unknowable up front (most commands).
    pub size: Option<u64>,
    pub default_on: bool,
    pub note: String,
    /// Needs administrator rights (system caches, snapshots, …).
    pub admin: bool,
    /// Move to the Trash instead of deleting (the user's own files).
    pub trash: bool,
}

impl Target {
    pub fn new(group: &'static str, category: impl Into<String>, name: impl Into<String>, action: Action) -> Self {
        let category = category.into();
        let name = name.into();
        let id = match &action {
            Action::Delete { paths } if paths.len() == 1 => format!("{group}:{}", paths[0].display()),
            Action::Command { command } => format!("cmd:{command}"),
            _ => format!("{group}:{category}:{name}"),
        };
        Target { id, group, category, name, action, size: None, default_on: true, note: String::new(), admin: false, trash: false }
    }

    pub fn on(mut self, on: bool) -> Self {
        self.default_on = on;
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    pub fn admin(mut self, admin: bool) -> Self {
        self.admin = admin;
        self
    }

    pub fn trash(mut self, trash: bool) -> Self {
        self.trash = trash;
        self
    }

    pub fn bytes(&self) -> u64 {
        self.size.unwrap_or(0)
    }

    pub fn paths(&self) -> &[PathBuf] {
        match &self.action {
            Action::Delete { paths } => paths,
            Action::Command { .. } => &[],
        }
    }
}
