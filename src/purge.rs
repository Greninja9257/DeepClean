//! Finds rebuildable project artifacts (node_modules, target, build, .venv, …)
//! with a parallel directory walk.

use crate::fsutil::{FILES_SEEN, age_days, tilde};
use crate::model::{Action, Target, group};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;

struct Rule {
    /// Directory names; a trailing `*` makes it a prefix match.
    names: &'static [&'static str],
    /// The parent must contain one of these (`*.ext` = any file with that
    /// extension). Empty = no requirement.
    markers: &'static [&'static str],
    /// A file that must exist inside the artifact itself.
    inside: Option<&'static str>,
    /// Pre-selected in the picker. Off for names that are sometimes real
    /// shipped output (e.g. `dist` in a downloaded project).
    on: bool,
}

const JS: &[&str] = &["package.json"];
const GRADLE: &[&str] = &[
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
];
const PY: &[&str] = &[
    "pyproject.toml",
    "requirements.txt",
    "setup.py",
    "setup.cfg",
    "Pipfile",
    "poetry.lock",
    "uv.lock",
];
const DOTNET: &[&str] = &["*.csproj", "*.fsproj", "*.vbproj"];

const RULES: &[Rule] = &[
    Rule {
        names: &["node_modules"],
        markers: &[],
        inside: None,
        on: true,
    },
    Rule {
        names: &["target"],
        markers: &["Cargo.toml", "pom.xml", "build.sbt"],
        inside: None,
        on: true,
    },
    Rule {
        names: &[".gradle", ".cxx", ".externalNativeBuild"],
        markers: GRADLE,
        inside: None,
        on: true,
    },
    Rule {
        names: &["build"],
        markers: GRADLE,
        inside: None,
        on: true,
    },
    Rule {
        names: &["build"],
        markers: &["pubspec.yaml", "CMakeLists.txt"],
        inside: None,
        on: true,
    },
    Rule {
        names: &["build", "dist"],
        markers: &["package.json", "setup.py", "pyproject.toml"],
        inside: None,
        on: false,
    },
    Rule {
        names: &[
            ".next",
            ".nuxt",
            ".output",
            ".svelte-kit",
            ".turbo",
            ".parcel-cache",
            ".angular",
            ".expo",
            ".docusaurus",
            ".vite",
            ".astro",
            "storybook-static",
        ],
        markers: JS,
        inside: None,
        on: true,
    },
    Rule {
        names: &[
            "__pycache__",
            ".pytest_cache",
            ".mypy_cache",
            ".ruff_cache",
            ".tox",
            ".nox",
            ".hypothesis",
        ],
        markers: &[],
        inside: None,
        on: true,
    },
    Rule {
        names: &[".venv", "venv", ".env"],
        markers: PY,
        inside: Some("pyvenv.cfg"),
        on: true,
    },
    Rule {
        names: &["Pods"],
        markers: &["Podfile"],
        inside: None,
        on: true,
    },
    Rule {
        names: &[".build", ".swiftpm"],
        markers: &["Package.swift"],
        inside: None,
        on: true,
    },
    Rule {
        names: &[".dart_tool"],
        markers: &["pubspec.yaml"],
        inside: None,
        on: true,
    },
    Rule {
        names: &["zig-cache", ".zig-cache", "zig-out"],
        markers: &["build.zig"],
        inside: None,
        on: true,
    },
    Rule {
        names: &[".stack-work"],
        markers: &["stack.yaml"],
        inside: None,
        on: true,
    },
    Rule {
        names: &["dist-newstyle"],
        markers: &["cabal.project", "*.cabal"],
        inside: None,
        on: true,
    },
    Rule {
        names: &["_build", "deps"],
        markers: &["mix.exs"],
        inside: None,
        on: true,
    },
    Rule {
        names: &["_build"],
        markers: &["dune-project"],
        inside: None,
        on: true,
    },
    Rule {
        names: &["bin", "obj"],
        markers: DOTNET,
        inside: None,
        on: true,
    },
    Rule {
        names: &[".terraform"],
        markers: &["*.tf"],
        inside: None,
        on: true,
    },
    Rule {
        names: &["elm-stuff"],
        markers: &["elm.json"],
        inside: None,
        on: true,
    },
    Rule {
        names: &["cmake-build-*"],
        markers: &["CMakeLists.txt"],
        inside: None,
        on: true,
    },
    // Game engines: Unity's Library is a pure import cache (often 5–20 GB).
    Rule { names: &["Library", "Temp"], markers: &["ProjectSettings"], inside: None, on: true },
    Rule { names: &[".godot", ".import"], markers: &["project.godot"], inside: None, on: true },
    Rule { names: &["Intermediate", "DerivedDataCache"], markers: &["*.uproject"], inside: None, on: true },
    // Misc tooling output
    Rule { names: &[".nyc_output", "coverage", ".cache"], markers: JS, inside: None, on: true },
    Rule { names: &[".ipynb_checkpoints"], markers: &[], inside: None, on: true },
    Rule { names: &["cdk.out"], markers: &["cdk.json"], inside: None, on: true },
    Rule { names: &[".serverless"], markers: &["serverless.yml", "serverless.yaml"], inside: None, on: true },
    Rule { names: &[".aws-sam"], markers: &["samconfig.toml", "template.yaml"], inside: None, on: true },
    Rule { names: &[".direnv"], markers: &[".envrc"], inside: None, on: true },
    Rule { names: &[".pixi"], markers: &["pixi.toml"], inside: None, on: true },
];

/// Directories never descended into while hunting for projects.
fn skip_descent(name: &str, depth: usize) -> bool {
    (name.starts_with('.'))
        || (depth == 0
            && matches!(
                name,
                "Library" | "Applications" | "Pictures" | "Music" | "Movies"
            ))
        || [
            ".app",
            ".photoslibrary",
            ".musiclibrary",
            ".tvlibrary",
            ".xcarchive",
            ".bundle",
            ".framework",
            ".photolibrary",
            ".fcpbundle",
            ".logicx",
        ]
        .iter()
        .any(|ext| name.ends_with(ext))
}

fn name_matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => pattern == name,
    }
}

fn has_marker(markers: &[&str], files: &HashSet<String>) -> bool {
    markers.is_empty()
        || markers.iter().any(|m| match m.strip_prefix('*') {
            Some(ext) => files.iter().any(|f| f.ends_with(ext)),
            None => files.contains(*m),
        })
}

struct Found {
    path: PathBuf,
    in_repo: bool,
    on: bool,
}

fn walk(dir: &Path, depth: usize, in_repo: bool, max_depth: usize) -> Vec<Found> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = HashSet::new();
    let mut dirs = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        match e.file_type() {
            Ok(ft) if ft.is_dir() => dirs.push(name),
            Ok(_) => {
                files.insert(name);
            }
            Err(_) => {}
        }
    }
    FILES_SEEN.fetch_add((files.len() + dirs.len()) as u64, Ordering::Relaxed);
    let in_repo = in_repo || files.contains(".git") || dirs.iter().any(|d| d == ".git");
    // markers may be files or folders (e.g. Unity's ProjectSettings/)
    let mut names = files;
    names.extend(dirs.iter().cloned());

    let mut found = Vec::new();
    let mut descend = Vec::new();
    for name in dirs {
        let path = dir.join(&name);
        let rule = RULES.iter().find(|r| {
            r.names.iter().any(|n| name_matches(n, &name))
                && has_marker(r.markers, &names)
                && r.inside.is_none_or(|f| path.join(f).exists())
        });
        if let Some(rule) = rule {
            found.push(Found {
                path,
                in_repo,
                on: rule.on,
            });
        } else if depth < max_depth && !skip_descent(&name, depth) {
            descend.push(path);
        }
    }
    found.extend(
        descend
            .par_iter()
            .flat_map_iter(|d| walk(d, depth + 1, in_repo, max_depth))
            .collect::<Vec<_>>(),
    );
    found
}

/// Mole-style guard: never purge a dir holding signing keys / secrets.
fn holds_secrets(dir: &Path) -> bool {
    const EXT: &[&str] = &[
        "pem",
        "key",
        "p12",
        "pfx",
        "keystore",
        "jks",
        "mobileprovision",
        "p8",
    ];
    let Ok(rd) = fs::read_dir(dir) else {
        return false;
    };
    rd.flatten().any(|e| {
        let p = e.path();
        p.extension()
            .is_some_and(|x| EXT.contains(&x.to_string_lossy().to_lowercase().as_str()))
    })
}

/// True if git tracks anything inside `path` (i.e. it's source, not output).
fn git_tracked(path: &Path) -> bool {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return true;
    };
    // fsmonitor off: with core.fsmonitor=true in the user's config, every
    // `git ls-files` would otherwise leave a background daemon per repo.
    let child = Command::new("git")
        .args(["-c", "core.fsmonitor=false"])
        .arg("-C")
        .arg(parent)
        .args(["ls-files", "-z", "--"])
        .arg(name)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return false };
    let mut buf = [0u8; 1];
    let tracked = child
        .stdout
        .take()
        .is_some_and(|mut o| o.read(&mut buf).unwrap_or(0) > 0);
    let _ = child.kill();
    let _ = child.wait();
    tracked
}

pub struct PurgeOptions {
    pub roots: Vec<PathBuf>,
    pub min_age_days: u64,
    pub max_depth: usize,
}

pub struct PurgeScan {
    pub targets: Vec<Target>,
    pub skipped_recent: usize,
    pub skipped_protected: usize,
}

pub fn scan(home: &Path, opts: &PurgeOptions) -> PurgeScan {
    let found: Vec<Found> = opts
        .roots
        .par_iter()
        .flat_map_iter(|r| walk(r, 0, false, opts.max_depth))
        .collect();

    // de-dup when roots overlap
    let mut seen = HashSet::new();
    let found: Vec<Found> = found
        .into_iter()
        .filter(|f| seen.insert(f.path.clone()))
        .collect();

    let checked: Vec<(Found, u64, Option<&str>)> = found
        .into_par_iter()
        .map(|f| {
            let age = age_days(&f.path);
            let verdict = if age < opts.min_age_days {
                Some("recent")
            } else if holds_secrets(&f.path) || (f.in_repo && git_tracked(&f.path)) {
                Some("protected")
            } else {
                None
            };
            (f, age, verdict)
        })
        .collect();

    let skipped_recent = checked.iter().filter(|c| c.2 == Some("recent")).count();
    let skipped_protected = checked.iter().filter(|c| c.2 == Some("protected")).count();
    let targets = checked
        .into_iter()
        .filter(|c| c.2.is_none())
        .map(|(f, age, _)| {
            let category = tilde(f.path.parent().unwrap(), home);
            let name = f.path.file_name().unwrap().to_string_lossy().to_string();
            Target::new(group::PROJECTS, category, name, Action::Delete { paths: vec![f.path] })
                .on(f.on)
                .note(format!("{age} days old"))
        })
        .collect();
    PurgeScan {
        targets,
        skipped_recent,
        skipped_protected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_only_real_artifacts() {
        let root = std::env::temp_dir().join(format!("deepclean-purge-{}", std::process::id()));
        let mk = |p: &str| fs::create_dir_all(root.join(p)).unwrap();
        let touch = |p: &str| fs::write(root.join(p), b"").unwrap();
        mk("rust/target/debug");
        touch("rust/Cargo.toml");
        mk("notrust/target"); // no Cargo.toml → keep
        mk("web/node_modules/x/node_modules"); // nested: report only the outer
        touch("web/package.json");
        mk("py/.venv");
        touch("py/.venv/pyvenv.cfg");
        touch("py/pyproject.toml");
        mk("py2/.venv"); // no pyvenv.cfg → keep
        touch("py2/pyproject.toml");

        let mut got: Vec<String> = walk(&root, 1, false, 10)
            .into_iter()
            .map(|f| f.path.strip_prefix(&root).unwrap().display().to_string())
            .collect();
        got.sort();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(got, ["py/.venv", "rust/target", "web/node_modules"]);
    }
}
