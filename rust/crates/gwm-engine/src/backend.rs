//! Where a task's script runs: a Docker or Apptainer container, or (tests only)
//! straight on this computer. The engine always runs on Linux or macOS (on
//! Windows inside WSL), so a path is the same inside and outside the container.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

/// One task ready to run: `.command.sh` is written in `dir`.
#[derive(Debug, Clone)]
pub struct TaskRun {
    /// Container name (unique per task, so a stop can kill it)
    pub name: String,
    pub dir: PathBuf,
    pub image: String,
    pub cpus: u32,
    pub memory_gb: u32,
    /// Folders the task needs, mounted at the same path: (path, read-only)
    pub mounts: Vec<(PathBuf, bool)>,
}

pub trait Backend: Sync {
    /// "docker", "apptainer" or "local"
    fn name(&self) -> &str;
    /// Run the task to the end (or until `kill` is set); returns the exit code.
    fn run(&self, task: &TaskRun, kill: &AtomicBool) -> Result<i32>;
    /// Registry digest of an image, for provenance.
    fn image_digest(&self, _image: &str) -> Option<String> {
        None
    }
}

fn wait(mut child: Child, kill: &AtomicBool, on_kill: impl Fn()) -> Result<i32> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.code().unwrap_or(-1));
        }
        if kill.load(Ordering::Relaxed) {
            on_kill();
            let _ = child.kill();
            let status = child.wait()?;
            return Ok(status.code().unwrap_or(-1));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn logs(dir: &Path) -> Result<(Stdio, Stdio)> {
    Ok((
        File::create(dir.join(".command.out"))?.into(),
        File::create(dir.join(".command.err"))?.into(),
    ))
}

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).stderr(Stdio::null()).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(unix)]
fn user() -> String {
    // SAFETY: getuid/getgid have no preconditions and can't fail
    unsafe { format!("{}:{}", libc::getuid(), libc::getgid()) }
}

#[cfg(not(unix))]
fn user() -> String {
    "0:0".into()
}

// ---------------------------------------------------------------- docker

/// Docker Engine (Linux, WSL), Colima or Docker Desktop: the `docker` command.
pub struct Docker {
    pub program: String,
}

impl Default for Docker {
    fn default() -> Self {
        Docker {
            program: "docker".into(),
        }
    }
}

impl Backend for Docker {
    fn name(&self) -> &str {
        "docker"
    }

    fn run(&self, t: &TaskRun, kill: &AtomicBool) -> Result<i32> {
        let mut cmd = Command::new(&self.program);
        cmd.args(["run", "-i", "--rm", "--name", &t.name, "-u", &user()]);
        // same as Nextflow: CPU shares (not a hard cap), a hard memory limit
        cmd.args([
            "--cpu-shares",
            &(1024 * t.cpus).to_string(),
            "--memory",
            &format!("{}g", t.memory_gb),
        ]);
        for (p, ro) in &t.mounts {
            let p = p.to_string_lossy();
            cmd.args(["-v", &if *ro { format!("{p}:{p}:ro") } else { format!("{p}:{p}") }]);
        }
        cmd.args([
            "-w",
            &t.dir.to_string_lossy(),
            &t.image,
            "/bin/bash",
            "-ue",
            ".command.sh",
        ]);
        let (out, err) = logs(&t.dir)?;
        let child = cmd
            .current_dir(&t.dir)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .context("starting docker")?;
        wait(child, kill, || {
            let _ = Command::new(&self.program)
                .args(["kill", &t.name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        })
    }

    fn image_digest(&self, image: &str) -> Option<String> {
        output(
            &self.program,
            &["image", "inspect", "--format", "{{index .RepoDigests 0}}", image],
        )
    }
}

// ---------------------------------------------------------------- apptainer

/// Apptainer (Linux servers and HPC); pulls the same Docker images.
pub struct Apptainer {
    pub program: String,
}

impl Default for Apptainer {
    fn default() -> Self {
        Apptainer {
            program: "apptainer".into(),
        }
    }
}

impl Backend for Apptainer {
    fn name(&self) -> &str {
        "apptainer"
    }

    fn run(&self, t: &TaskRun, kill: &AtomicBool) -> Result<i32> {
        let mut cmd = Command::new(&self.program);
        cmd.args(["exec", "--no-home", "--pwd", &t.dir.to_string_lossy()]);
        for (p, ro) in &t.mounts {
            let p = p.to_string_lossy();
            cmd.args(["--bind", &if *ro { format!("{p}:{p}:ro") } else { format!("{p}:{p}") }]);
        }
        cmd.args([&format!("docker://{}", t.image), "/bin/bash", "-ue", ".command.sh"]);
        let (out, err) = logs(&t.dir)?;
        let child = cmd
            .current_dir(&t.dir)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .context("starting apptainer")?;
        wait(child, kill, || {})
    }
}

// ---------------------------------------------------------------- local

/// Runs the script with this computer's bash, no container. For tests.
pub struct Local;

impl Backend for Local {
    fn name(&self) -> &str {
        "local"
    }

    fn run(&self, t: &TaskRun, kill: &AtomicBool) -> Result<i32> {
        let (out, err) = logs(&t.dir)?;
        let child = Command::new("bash")
            .args(["-ue", ".command.sh"])
            .current_dir(&t.dir)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .context("starting bash")?;
        wait(child, kill, || {})
    }
}

/// Docker when it is there, else Apptainer (same rule as v1).
pub fn detect() -> Box<dyn Backend> {
    if output("docker", &["--version"]).is_some() {
        Box::new(Docker::default())
    } else if output("apptainer", &["--version"]).is_some() {
        Box::new(Apptainer::default())
    } else {
        Box::new(Docker::default())
    }
}
