//! Where the shipped files (ROOT) and the user's data (HOME) live.
//!
//! The app was called NTG Pathology Genomics Workflow Pipeline up to v1 (data
//! in `~/.ntg`, `NTG_*` variables). Those still work: the `NTG_*` variables are
//! read when the `GWM_*` ones are not set, and [`migrate_home`] moves `~/.ntg`.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// `GWM_<name>`, else the v1 `NTG_<name>`.
fn var(name: &str) -> Option<OsString> {
    env::var_os(format!("GWM_{name}")).or_else(|| env::var_os(format!("NTG_{name}")))
}

/// The folder with `tools/`, `workflows/` and `databases/catalog.yml`.
/// `GWM_ROOT` wins; otherwise the first folder holding them above the running
/// program (installed app: its resources folder) or the working directory.
pub fn root() -> PathBuf {
    if let Some(r) = var("ROOT") {
        return PathBuf::from(r);
    }
    let mut starts = Vec::new();
    if let Ok(exe) = env::current_exe() {
        starts.push(exe);
    }
    if let Ok(cwd) = env::current_dir() {
        starts.push(cwd);
    }
    starts.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    for start in starts {
        for dir in start.ancestors() {
            if is_root(dir) {
                return dir.to_path_buf();
            }
        }
    }
    PathBuf::from(".")
}

fn is_root(dir: &Path) -> bool {
    dir.join("tools").is_dir() && dir.join("workflows").is_dir() && dir.join("databases/catalog.yml").is_file()
}

fn user_home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// The user's data folder: `GWM_HOME`, else `~/.gwm`.
pub fn home() -> PathBuf {
    var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home().join(".gwm"))
}

/// Reference databases: `GWM_DB_DIR`, else `<home>/databases`.
pub fn db_dir(home: &Path) -> PathBuf {
    var("DB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("databases"))
}

/// First start after the rename: move the v1 data folder `~/.ntg` to `~/.gwm`
/// (workflows, packs, runs and databases come along) and leave `~/.ntg` as a
/// link to it, so a v1 install on the same computer keeps working.
/// Does nothing when a data folder is set by a variable or `~/.gwm` exists.
pub fn migrate_home() -> io::Result<Option<PathBuf>> {
    if var("HOME").is_some() {
        return Ok(None);
    }
    migrate(&user_home().join(".ntg"), &user_home().join(".gwm"))
}

fn migrate(old: &Path, new: &Path) -> io::Result<Option<PathBuf>> {
    if new.exists() || !old.is_dir() || fs::symlink_metadata(old)?.file_type().is_symlink() {
        return Ok(None);
    }
    fs::rename(old, new)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(new, old)?;
    Ok(Some(new.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_old_folder_once() {
        let base = env::temp_dir().join(format!("gwm-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (old, new) = (base.join(".ntg"), base.join(".gwm"));
        fs::create_dir_all(old.join("workflows")).unwrap();
        assert_eq!(migrate(&old, &new).unwrap(), Some(new.clone()));
        assert!(new.join("workflows").is_dir());
        #[cfg(unix)]
        assert!(old.join("workflows").is_dir(), "old path still works through the link");
        assert_eq!(migrate(&old, &new).unwrap(), None, "second start: nothing to do");
        fs::remove_dir_all(&base).ok();
    }
}
