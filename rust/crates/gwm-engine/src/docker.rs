//! Finding Docker without the user's help.
//!
//! A Mac app opened from Finder or the Dock gets a bare `PATH`
//! (`/usr/bin:/bin:/usr/sbin:/sbin`), not the one from the user's shell, so a
//! `docker` installed by Homebrew, Docker Desktop, Colima or Rancher Desktop is
//! not found. [`extend_path`] adds the usual install folders. And when the
//! `docker` command's own default does not answer, [`probe`] tries the sockets
//! Colima, Rancher Desktop and Docker Desktop make; the one that answers is
//! then used for every `docker` call of this process ([`command`], [`apply`]).

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// `DOCKER_HOST` for this process's docker calls, when the default did not answer.
static HOST: Mutex<Option<String>> = Mutex::new(None);

fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// Folders `docker` is usually installed in, beyond the system's own.
fn extra_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        // Homebrew on Apple silicon
        "/opt/homebrew/bin".into(),
        // Homebrew on Intel, Docker Desktop's link, tools installed by hand
        "/usr/local/bin".into(),
        // Docker Desktop
        home.join(".docker/bin"),
        "/Applications/Docker.app/Contents/Resources/bin".into(),
        home.join(".colima/bin"),
        // Rancher Desktop
        home.join(".rd/bin"),
        // MacPorts
        "/opt/local/bin".into(),
    ]
}

/// `current` with each of `extra` that exists added at the end: what the user
/// set up themself stays first.
fn merged(mut current: Vec<PathBuf>, extra: Vec<PathBuf>, exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    for p in extra {
        if !current.contains(&p) && exists(&p) {
            current.push(p);
        }
    }
    current
}

/// Add the usual install folders to `PATH`. Call first thing in `main`, before
/// any thread or child process is started.
pub fn extend_path() {
    if !cfg!(unix) {
        return;
    }
    let current: Vec<PathBuf> = env::var_os("PATH")
        .map(|p| env::split_paths(&p).collect())
        .unwrap_or_default();
    let n = current.len();
    let paths = merged(current, extra_dirs(&home()), |p| p.is_dir());
    if paths.len() > n {
        if let Ok(joined) = env::join_paths(paths) {
            env::set_var("PATH", joined);
        }
    }
}

/// Sockets to try when the default does not answer.
fn sockets(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".colima/default/docker.sock"),
        home.join(".colima/docker.sock"),
        home.join(".rd/docker.sock"),
        home.join(".docker/run/docker.sock"),
    ]
}

/// What was found of Docker on this computer.
#[derive(Debug, Clone, Default)]
pub struct Probe {
    /// the `docker` command is there
    pub installed: bool,
    /// the server's version, when it answers
    pub version: Option<String>,
    /// Docker's own words when it did not answer
    pub detail: Option<String>,
}

impl Probe {
    /// Docker is there and running, but this user may not talk to it (Linux:
    /// not in the `docker` group yet, or not logged in again since).
    pub fn denied(&self) -> bool {
        self.detail
            .as_deref()
            .is_some_and(|d| d.to_lowercase().contains("permission denied"))
    }
}

fn installed() -> bool {
    Command::new("docker")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The server's version, or Docker's own error.
fn info(host: Option<&str>) -> Result<String, String> {
    let mut cmd = Command::new("docker");
    cmd.args(["info", "--format", "{{.ServerVersion}}"])
        .stdin(Stdio::null());
    if let Some(h) = host {
        cmd.env("DOCKER_HOST", h);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !text.is_empty() {
        return Ok(text);
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if err.is_empty() { text } else { err })
}

/// Look for Docker now. Remembers the socket that answered, for [`command`].
pub fn probe() -> Probe {
    if !installed() {
        return Probe::default();
    }
    let ok = |version: String| Probe {
        installed: true,
        version: Some(version),
        detail: None,
    };
    // the socket that answered last time, as long as it still does
    let remembered = HOST.lock().unwrap().clone();
    if let Some(h) = remembered {
        if let Ok(v) = info(Some(&h)) {
            return ok(v);
        }
        *HOST.lock().unwrap() = None;
    }
    let first = match info(None) {
        Ok(v) => return ok(v),
        Err(e) => e,
    };
    // a DOCKER_HOST the user set themself is theirs: nothing else is tried
    if env::var_os("DOCKER_HOST").is_none() {
        for sock in sockets(&home()) {
            if !sock.exists() {
                continue;
            }
            let h = format!("unix://{}", sock.display());
            if let Ok(v) = info(Some(&h)) {
                *HOST.lock().unwrap() = Some(h);
                return ok(v);
            }
        }
    }
    Probe {
        installed: true,
        version: None,
        detail: Some(first).filter(|d| !d.is_empty()),
    }
}

/// GB of memory Docker's own machine has, when it differs from this computer's:
/// on a Mac the containers live in a Linux machine (Colima, Docker Desktop)
/// with a share of the Mac's memory. Nothing when Docker does not answer.
pub fn memory_gb() -> Option<u32> {
    let out = command()
        .args(["info", "--format", "{{.MemTotal}}"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let bytes: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    whole_gb(bytes)
}

/// A machine given 12 GB reports a little less (its own kernel takes some):
/// to the nearest GB, so that still counts as 12.
fn whole_gb(bytes: u64) -> Option<u32> {
    let gb = (bytes as f64 / 1_073_741_824.0).round() as u32;
    (gb > 0).then_some(gb)
}

/// Point a command that runs `docker` (itself or through a shell) at the
/// Docker that answered.
pub fn apply(cmd: &mut Command) {
    if let Some(h) = HOST.lock().unwrap().as_deref() {
        cmd.env("DOCKER_HOST", h);
    }
}

/// A `docker` command aimed at the Docker that answered.
pub fn command() -> Command {
    let mut cmd = Command::new("docker");
    apply(&mut cmd);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_gets_the_folders_that_exist_after_the_users_own() {
        let current = vec![PathBuf::from("/usr/bin"), PathBuf::from("/usr/local/bin")];
        let home = Path::new("/Users/me");
        let there = |p: &Path| p == Path::new("/opt/homebrew/bin") || p == Path::new("/Users/me/.rd/bin");
        let got = merged(current, extra_dirs(home), there);
        assert_eq!(
            got,
            ["/usr/bin", "/usr/local/bin", "/opt/homebrew/bin", "/Users/me/.rd/bin"]
                .map(PathBuf::from)
                .to_vec()
        );
    }

    #[test]
    fn path_is_left_alone_when_nothing_new_exists() {
        let current = vec![PathBuf::from("/usr/bin")];
        assert_eq!(merged(current.clone(), extra_dirs(Path::new("/h")), |_| false), current);
    }

    #[test]
    fn docker_memory_is_counted_in_whole_gb() {
        // a Colima machine given 12 GB reports about 11.7
        assert_eq!(whole_gb(12_540_000_000), Some(12));
        assert_eq!(whole_gb(28 * 1_073_741_824), Some(28));
        assert_eq!(whole_gb(0), None);
    }

    #[test]
    fn permission_denied_is_told_apart_from_not_running() {
        let p = |d: &str| Probe {
            installed: true,
            version: None,
            detail: Some(d.into()),
        };
        assert!(p("Got permission denied while trying to connect to the Docker daemon socket").denied());
        assert!(!p("Cannot connect to the Docker daemon at unix:///var/run/docker.sock").denied());
    }
}
