//! "Remove everything" (Settings): what the app keeps on this computer beside
//! the program itself, how big it is, and removing it before an uninstall.
//!
//! Two things are the engine's to remove: the tool containers (Docker images
//! of the tools, and images the app built) and the data folder (workflows,
//! added tools, databases, working files). Run folders are never touched.
//! Docker itself and the WSL Ubuntu are the desktop app's (Windows only).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Result};
use gwm_core::{paths, Catalog};
use gwm_engine::runs;
use serde_json::{json, Value};

/// Images the app built itself for tools without a ready-made container.
const LOCAL_PREFIX: &str = "gwm-local/";

fn docker(args: &[&str]) -> Option<String> {
    let out = Command::new("docker").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Every container the app's tools name (all versions).
fn tool_images(cat: &Catalog) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for tool in cat.load_tools().unwrap_or_default().values() {
        for t in std::iter::once(tool).chain(tool.variants.values()) {
            if !t.container.trim().is_empty() {
                out.insert(t.container.trim().to_string());
            }
        }
    }
    out
}

/// Of the images Docker has here, the ones that are this app's.
fn ours(present: &str, wanted: &BTreeSet<String>) -> Vec<String> {
    present
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.ends_with(":<none>"))
        .filter(|l| wanted.contains(*l) || l.starts_with(LOCAL_PREFIX))
        .map(str::to_string)
        .collect()
}

fn installed_images(cat: &Catalog) -> Option<Vec<String>> {
    let present = docker(&["image", "ls", "--format", "{{.Repository}}:{{.Tag}}"])?;
    Some(ours(&present, &tool_images(cat)))
}

/// Bytes under `dir`; links are counted as links, not followed.
fn folder_bytes(dir: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_bytes(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// The data folder, and the databases folder when it is kept somewhere else.
fn data_folders(cat: &Catalog) -> Vec<PathBuf> {
    let mut out = vec![cat.home.clone()];
    let db = paths::db_dir(&cat.home);
    if !db.starts_with(&cat.home) {
        out.push(db);
    }
    out
}

/// Runs that are going or waiting: nothing is removed under them.
fn active_runs(cat: &Catalog) -> usize {
    runs::list(cat)
        .iter()
        .filter(|r| {
            let status = match (r["status"].as_str(), r["id"].as_str()) {
                // a run whose process is gone is not running any more
                (Some("running"), Some(id)) => runs::refresh(cat, id).map(|f| f["status"].clone()).unwrap_or_default(),
                _ => r["status"].clone(),
            };
            status == "running" || status == "queued"
        })
        .count()
}

/// What could be removed, with sizes (containers: the sum of the images, so a
/// little more than the disk they really take, since images share parts).
pub fn overview(cat: &Catalog) -> Value {
    let images = installed_images(cat);
    let image_bytes: u64 = images
        .as_deref()
        .filter(|i| !i.is_empty())
        .and_then(|i| {
            let mut args = vec!["image", "inspect", "--format", "{{.Size}}"];
            args.extend(i.iter().map(String::as_str));
            docker(&args)
        })
        .map(|text| text.lines().filter_map(|l| l.trim().parse::<u64>().ok()).sum())
        .unwrap_or(0);
    let folders = data_folders(cat);
    json!({
        "active_runs": active_runs(cat),
        "containers": {
            "available": images.is_some(),
            "count": images.as_ref().map_or(0, Vec::len),
            "bytes": image_bytes,
        },
        "data": {
            "folders": folders,
            "bytes": folders.iter().map(|f| folder_bytes(f)).sum::<u64>(),
        },
        "runs_folder": cat.runs,
    })
}

/// Remove the ticked things. Containers go first: which ones are ours is read
/// from the tools in the data folder.
pub fn remove(cat: &Catalog, containers: bool, data: bool) -> Result<Value> {
    let going = active_runs(cat);
    if going > 0 {
        bail!("{going} run(s) are still going. Wait for them to finish, or stop them, then try again.");
    }
    let mut removed_images = 0;
    let mut problems = Vec::new();
    if containers {
        for image in installed_images(cat).unwrap_or_default() {
            match Command::new("docker").args(["image", "rm", "--force", &image]).output() {
                Ok(out) if out.status.success() => removed_images += 1,
                Ok(out) => problems.push(format!("{image}: {}", String::from_utf8_lossy(&out.stderr).trim())),
                Err(e) => problems.push(format!("{image}: {e}")),
            }
        }
    }
    if data {
        for folder in data_folders(cat) {
            if let Err(e) = empty_folder(&folder) {
                problems.push(format!("{}: {e}", folder.display()));
            }
        }
        // the link version 1 left to the data folder (`~/.ntg`) would point nowhere
        if let Some(old) = paths::user_home_dir().map(|h| h.join(".ntg")) {
            if fs::symlink_metadata(&old).is_ok_and(|m| m.file_type().is_symlink()) && !old.exists() {
                let _ = fs::remove_file(&old);
            }
        }
    }
    Ok(json!({"containers_removed": removed_images, "data_removed": data, "problems": problems}))
}

/// Delete everything inside `dir` and then `dir` itself. On Windows with WSL
/// the engine runs from a copy in the data folder: Linux lets a running
/// program's file be deleted, so that is fine.
fn empty_folder(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    // refuse anything that could not be a data folder (a home folder, a drive)
    if dir.parent().is_none() || paths::user_home_dir().is_some_and(|h| h == dir) {
        return Err(std::io::Error::other("not removing a home folder or a whole disk"));
    }
    fs::remove_dir_all(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_only_the_apps_images() {
        let wanted: BTreeSet<String> = ["quay.io/biocontainers/fastp:0.24.0--heae3180_1".to_string()].into();
        let present = "quay.io/biocontainers/fastp:0.24.0--heae3180_1\nubuntu:24.04\ngwm-local/nanostat:1.6.0-abc1234\n<none>:<none>\n\n";
        assert_eq!(
            ours(present, &wanted),
            [
                "quay.io/biocontainers/fastp:0.24.0--heae3180_1",
                "gwm-local/nanostat:1.6.0-abc1234"
            ]
        );
    }

    #[test]
    fn counts_and_empties_a_folder() {
        let base = std::env::temp_dir().join(format!("gwm-cleanup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("tools/a")).unwrap();
        fs::write(base.join("tools/a/tool.yml"), "12345").unwrap();
        fs::write(base.join("x.json"), "123").unwrap();
        assert_eq!(folder_bytes(&base), 8);
        empty_folder(&base).unwrap();
        assert!(!base.exists());
        empty_folder(&base).unwrap();
    }

    #[test]
    fn never_empties_a_home_folder() {
        if let Some(home) = paths::user_home_dir() {
            assert!(empty_folder(&home).is_err());
        }
    }
}
