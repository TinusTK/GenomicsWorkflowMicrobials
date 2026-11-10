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

/// The user's own home folder, when there is one.
pub fn user_home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// The user's data folder: `GWM_HOME`, else `~/.gwm`.
pub fn home() -> PathBuf {
    var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home().join(".gwm"))
}

/// Where run folders go: `GWM_RUNS_DIR`, else Documents › Genomics Workflow
/// Microbials (on Windows the Windows Documents folder, also when the engine
/// runs inside WSL). One folder per run, so it is easy to find and delete.
pub fn runs_root() -> PathBuf {
    if let Some(r) = var("RUNS_DIR") {
        return native_path(&r.to_string_lossy());
    }
    let docs = windows_documents()
        .or_else(dirs::document_dir)
        .unwrap_or_else(|| user_home().join("Documents"));
    docs.join("Genomics Workflow Microbials")
}

/// Inside WSL: the Windows user's Documents folder (as /mnt/c/...).
fn windows_documents() -> Option<PathBuf> {
    if !is_wsl() {
        return None;
    }
    let out = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Environment]::GetFolderPath('MyDocuments')",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then(|| native_path(&text))
}

pub fn is_wsl() -> bool {
    cfg!(target_os = "linux")
        && fs::read_to_string("/proc/version").is_ok_and(|v| v.to_lowercase().contains("microsoft"))
}

/// A path typed or picked on Windows (`C:\Lab\Reads`) as the engine sees it:
/// under WSL `/mnt/c/Lab/Reads`; anything else unchanged.
pub fn native_path(p: &str) -> PathBuf {
    let b = p.as_bytes();
    let drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    if cfg!(target_os = "linux") && drive {
        let rest = p[2..].replace('\\', "/");
        return PathBuf::from(format!("/mnt/{}{}", (b[0] as char).to_ascii_lowercase(), rest));
    }
    PathBuf::from(p)
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
    #[cfg(target_os = "linux")]
    fn windows_paths() {
        assert_eq!(native_path(r"C:\Lab\Reads 1"), PathBuf::from("/mnt/c/Lab/Reads 1"));
        assert_eq!(native_path("/home/lab"), PathBuf::from("/home/lab"));
    }

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
