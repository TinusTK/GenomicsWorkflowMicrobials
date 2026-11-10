//! Where the shipped files (ROOT) and the user's data (HOME) live.

use std::env;
use std::path::{Path, PathBuf};

/// The folder with `tools/`, `workflows/` and `databases/catalog.yml`.
/// `NTG_ROOT` wins; otherwise the first folder holding them above the running
/// program (installed app: its resources folder) or the working directory.
pub fn root() -> PathBuf {
    if let Some(r) = env::var_os("NTG_ROOT") {
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

/// The user's data folder: `NTG_HOME`, else `~/.ntg` (same as v1).
pub fn home() -> PathBuf {
    env::var_os("NTG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".ntg"))
}

/// Reference databases: `NTG_DB_DIR`, else `<home>/databases`.
pub fn db_dir(home: &Path) -> PathBuf {
    env::var_os("NTG_DB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("databases"))
}
