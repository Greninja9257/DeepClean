//! External cleanup commands. Requests only ever carry a command *id*; the
//! argv is rebuilt here, so nothing arbitrary can be executed.

use crate::model::{Action, Target, group};
use std::path::Path;
use std::process::Command;

pub fn in_path(binary: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(binary).is_file()))
        .unwrap_or(false)
        || ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
            .iter()
            .any(|d| Path::new(d).join(binary).is_file())
}

fn binary(name: &str) -> String {
    for d in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let p = Path::new(d).join(name);
        if p.is_file() {
            return p.display().to_string();
        }
    }
    name.to_string()
}

fn has_simctl() -> bool {
    Command::new("/usr/bin/xcrun")
        .args(["--find", "simctl"])
        .output()
        .is_ok_and(|o| o.status.success())
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

const LSREGISTER: &str =
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

/// Map a command id to the argv lists to run, or None if unknown.
pub fn resolve(id: &str) -> Option<Vec<Vec<String>>> {
    let v = |args: &[&str]| args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    Some(match id {
        "simctl-unavailable" => vec![v(&["/usr/bin/xcrun", "simctl", "delete", "unavailable"])],
        "brew-cleanup" => vec![v(&[&binary("brew"), "cleanup", "--prune=all"])],
        "conda-clean" => vec![v(&[&binary("conda"), "clean", "--all", "--yes"])],
        "docker-prune" => {
            let d = binary("docker");
            vec![v(&[&d, "system", "prune", "--force"]), v(&[&d, "builder", "prune", "--all", "--force"])]
        }
        "tm-snapshots" => vec![v(&["/usr/bin/tmutil", "deletelocalsnapshots", "/"])],
        // ── Optimize ──
        "flush-dns" => vec![v(&["/usr/bin/dscacheutil", "-flushcache"]), v(&["/usr/bin/killall", "-HUP", "mDNSResponder"])],
        "rebuild-launchservices" => vec![v(&[LSREGISTER, "-r", "-domain", "local", "-domain", "system", "-domain", "user"])],
        "reset-quicklook" => vec![v(&["/usr/bin/qlmanage", "-r", "cache"]), v(&["/usr/bin/qlmanage", "-r"])],
        "restart-dock" => vec![v(&["/usr/bin/killall", "Dock"])],
        "restart-finder" => vec![v(&["/usr/bin/killall", "Finder"])],
        "purge-memory" => vec![v(&["/usr/sbin/purge"])],
        "reindex-spotlight" => vec![v(&["/usr/bin/mdutil", "-E", "/"])],
        "verify-disk" => vec![v(&["/usr/sbin/diskutil", "verifyVolume", "/"])],
        _ => {
            let rt = id.strip_prefix("simruntime:").filter(|r| is_uuid(r))?;
            vec![v(&["/usr/bin/xcrun", "simctl", "runtime", "delete", rt])]
        }
    })
}

/// Commands worth offering on this machine.
pub fn discover(home: &Path, include_admin: bool) -> Vec<Target> {
    let cmd = |grp, cat: &str, name: &str, id: &str| {
        Target::new(grp, cat, name, Action::Command { command: id.into() })
    };
    let mut out = Vec::new();

    if home.join("Library/Developer/CoreSimulator/Devices").exists() && has_simctl() {
        out.push(cmd(group::DEV, "Xcode", "Delete unavailable simulators", "simctl-unavailable")
            .note("simulators for runtimes no longer installed"));
        out.extend(simulator_runtimes());
    }
    if in_path("brew") {
        out.push(cmd(group::DEV, "Native", "Homebrew: remove old versions", "brew-cleanup")
            .note("old formula versions & stale downloads"));
    }
    if in_path("conda") {
        out.push(cmd(group::DEV, "Python", "Conda: clean package cache", "conda-clean")
            .note("unused packages & tarballs"));
    }
    if in_path("docker") {
        out.push(cmd(group::DEV, "Docker", "Docker: prune build cache & dangling data", "docker-prune")
            .on(false)
            .note("needs Docker running; removes stopped containers"));
    }
    if include_admin {
        let n = local_snapshot_count();
        if n > 0 {
            out.push(cmd(group::SYSTEM, "Time Machine", "Local Time Machine snapshots", "tm-snapshots")
                .on(false)
                .admin(true)
                .note(format!("{n} snapshot(s) holding space from deleted files")));
        }
    }
    out
}

fn local_snapshot_count() -> usize {
    Command::new("/usr/bin/tmutil")
        .args(["listlocalsnapshots", "/"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.contains("com.apple.TimeMachine")).count())
        .unwrap_or(0)
}

/// Maintenance tasks for the Optimize screen: (id, name, description, admin, on by default).
pub fn optimize_tasks() -> Vec<Target> {
    const TASKS: &[(&str, &str, &str, bool, bool)] = &[
        ("flush-dns", "Flush DNS cache", "Fixes sites that won't load after network or DNS changes", true, false),
        ("reset-quicklook", "Reset QuickLook previews", "Rebuilds thumbnails and fixes blank or stale previews", false, true),
        ("rebuild-launchservices", "Rebuild Launch Services", "Removes duplicate apps from Open With menus", false, true),
        ("purge-memory", "Free up memory", "Releases inactive memory held by apps", true, false),
        ("restart-dock", "Restart Dock", "Fixes a frozen Dock, Launchpad or Mission Control", false, false),
        ("restart-finder", "Restart Finder", "Fixes Finder windows that stop updating", false, false),
        ("reindex-spotlight", "Rebuild Spotlight index", "Fixes missing search results. Reindexing takes a while", true, false),
        ("verify-disk", "Verify startup disk", "Checks the disk for errors without changing anything", true, false),
    ];
    TASKS
        .iter()
        .map(|(id, name, desc, admin, on)| {
            Target::new(group::OPTIMIZE, "Maintenance", *name, Action::Command { command: id.to_string() })
                .note(*desc)
                .admin(*admin)
                .on(*on)
        })
        .collect()
}

/// Installed iOS/watchOS/tvOS/visionOS simulator runtimes (several GB each).
fn simulator_runtimes() -> Vec<Target> {
    let Ok(out) = Command::new("/usr/bin/xcrun").args(["simctl", "runtime", "list", "-j"]).output() else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return Vec::new() };
    let Some(map) = json.as_object() else { return Vec::new() };
    map.values()
        .filter_map(|rt| {
            let id = rt.get("identifier")?.as_str()?;
            if !is_uuid(id) || rt.get("deletable").and_then(|d| d.as_bool()) == Some(false) {
                return None;
            }
            let platform = rt
                .get("platformIdentifier")
                .and_then(|p| p.as_str())
                .and_then(|p| p.rsplit('.').next())
                .unwrap_or("Simulator")
                .replace("simulator", "");
            let version = rt.get("version").and_then(|v| v.as_str()).unwrap_or("?");
            let mut t = Target::new(
                group::DEV,
                "Xcode",
                format!("{platform} {version} simulator runtime"),
                Action::Command { command: format!("simruntime:{id}") },
            )
            .on(false)
            .note("re-download from Xcode › Settings › Components");
            t.size = rt.get("sizeBytes").and_then(|s| s.as_u64());
            Some(t)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_commands_resolve() {
        assert!(resolve("brew-cleanup").is_some());
        assert!(resolve("simruntime:0A1B2C3D-0000-1111-2222-333344445555").is_some());
        assert!(resolve("simruntime:; rm -rf ~").is_none());
        assert!(resolve("rm -rf /").is_none());
        for t in optimize_tasks() {
            let crate::model::Action::Command { command } = &t.action else { panic!() };
            assert!(resolve(command).is_some(), "{command} must resolve");
        }
    }
}
