//! Space Lens: what's taking up room inside a folder.

use crate::fsutil::{self, InodeSet};
use rayon::prelude::*;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
    /// Direct children (folders only).
    pub items: u64,
}

#[derive(Serialize)]
pub struct Analysis {
    pub path: PathBuf,
    pub total: u64,
    pub entries: Vec<Entry>,
}

/// Size every direct child of `dir` in parallel, biggest first.
pub fn analyze(dir: &Path) -> Analysis {
    let seen = InodeSet::default();
    let children: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    let mut entries: Vec<Entry> = children
        .par_iter()
        .filter_map(|p| {
            let md = fs::symlink_metadata(p).ok()?;
            let is_dir = md.is_dir();
            Some(Entry {
                name: p.file_name()?.to_string_lossy().to_string(),
                path: p.clone(),
                size: fsutil::disk_usage(p, &seen),
                is_dir,
                items: if is_dir { fs::read_dir(p).map(|r| r.count() as u64).unwrap_or(0) } else { 0 },
            })
        })
        .filter(|e| e.path.to_str().is_some())
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.size));
    let total = entries.iter().map(|e| e.size).sum();
    Analysis { path: dir.to_path_buf(), total, entries }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_biggest_first() {
        let root = std::env::temp_dir().join(format!("deepclean-analyze-{}", std::process::id()));
        fs::create_dir_all(root.join("big")).unwrap();
        fs::write(root.join("big/f"), vec![1u8; 200_000]).unwrap();
        fs::write(root.join("small"), vec![1u8; 10]).unwrap();
        let a = analyze(&root);
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(a.entries[0].name, "big");
        assert_eq!(a.entries[0].items, 1);
        assert!(a.total >= 200_000);
    }
}
