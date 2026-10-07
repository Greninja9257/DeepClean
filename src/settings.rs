//! User preferences shared by the app and the CLI
//! (~/.config/deepclean/settings.json; the app writes it, the engine reads it).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Move the user's own files (downloads, large files, leftovers, uninstalled
    /// apps) to the Trash instead of deleting them. Caches are always deleted.
    pub trash_personal: bool,
    /// Skip project build folders modified within this many days.
    pub project_min_age_days: u64,
    /// Files at least this big are listed under Large Files.
    pub large_file_mb: u64,
    /// Downloads untouched this long are listed as old.
    pub old_download_days: u64,
    pub scan_large_files: bool,
    pub scan_leftovers: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            trash_personal: true,
            project_min_age_days: 7,
            large_file_mb: 200,
            old_download_days: 90,
            scan_large_files: true,
            scan_leftovers: true,
        }
    }
}

pub fn path(home: &Path) -> PathBuf {
    home.join(".config/deepclean/settings.json")
}

pub fn load(home: &Path) -> Settings {
    std::fs::read_to_string(path(home))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_keeps_defaults() {
        let s: Settings = serde_json::from_str(r#"{"large_file_mb": 500}"#).unwrap();
        assert_eq!(s.large_file_mb, 500);
        assert!(s.trash_personal);
        assert_eq!(s.project_min_age_days, 7);
    }
}
