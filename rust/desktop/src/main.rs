//! Genomics Workflow Microbials, the desktop app: the GUI in its own window.
//!
//! The engine (`gwm serve`, the app's local server) runs where Linux tools can:
//! on Mac and Linux straight from the app's own folder; on Windows inside WSL
//! (the Linux side of Windows), from the same installed folder. Runs are their
//! own processes, so closing the window never stops a run.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{Manager, RunEvent, State};

const PORT: u16 = 8642;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const NO_WINDOW: u32 = 0x0800_0000;

#[derive(Default)]
struct Backend {
    child: Mutex<Option<Child>>,
    /// why it isn't running: "wsl", "distro" (setup needed) or an error text
    problem: Mutex<Option<String>>,
    /// the app has a system tray icon: closing the window hides it there
    in_tray: Mutex<bool>,
    /// an update is being downloaded or installed
    updating: AtomicBool,
    /// bytes of the update downloaded so far, and its full size (0 until known)
    update_done: AtomicU64,
    update_total: AtomicU64,
}

#[derive(Serialize)]
struct UpdateProgress {
    done: u64,
    total: u64,
}

#[derive(Serialize)]
struct Status {
    running: bool,
    problem: Option<String>,
    windows: bool,
    distro: String,
    /// closing the window keeps the app in the system tray
    tray: bool,
}

fn answering() -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

fn distro() -> String {
    std::env::var("GWM_DISTRO").unwrap_or_else(|_| "Ubuntu-24.04".into())
}

fn runs_folder() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Documents"))
        .join("Genomics Workflow Microbials")
}

/// `C:\Program Files\X` as WSL sees it: `/mnt/c/Program Files/X`.
#[cfg_attr(not(windows), allow(dead_code))]
fn to_wsl(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.strip_prefix("//?/").unwrap_or(&s).to_string();
    match s.as_bytes() {
        [d, b':', ..] => format!("/mnt/{}{}", (*d as char).to_ascii_lowercase(), &s[2..]),
        _ => s,
    }
}

#[cfg(windows)]
fn wsl_distros() -> Option<Vec<String>> {
    wsl_list(&["-l", "-q"])
}

/// The app's Linux is switched on (it takes memory until it is switched off).
#[cfg(windows)]
fn distro_running() -> bool {
    let d = distro();
    wsl_list(&["-l", "-q", "--running"]).is_some_and(|l| l.iter().any(|x| x.eq_ignore_ascii_case(&d)))
}

#[cfg(windows)]
fn wsl_list(args: &[&str]) -> Option<Vec<String>> {
    let out = Command::new("wsl.exe").args(args).creation_flags(NO_WINDOW).output().ok()?;
    if !out.status.success() {
        return None;
    }
    // wsl.exe writes UTF-16
    let words: Vec<u16> = out.stdout.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&words).lines().map(|l| l.trim().trim_matches('\0').to_string()).filter(|l| !l.is_empty()).collect())
}

fn start(res: &Path) -> Result<Child, String> {
    #[cfg(windows)]
    {
        let d = distro();
        let list = wsl_distros().ok_or("wsl")?;
        if !list.iter().any(|l| l.eq_ignore_ascii_case(&d)) {
            return Err("distro".into());
        }
        // Run the engine from a copy on the Linux side, named after this build:
        // an app update can then replace the installed files while a run (its own
        // process, using the old copy) is still going.
        let src = res.join("bin").join("gwm-linux");
        let meta = std::fs::metadata(&src).map_err(|e| format!("The engine is missing from the app folder: {e}"))?;
        let stamp = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        let name = format!("gwm-{}-{}-{stamp}", env!("CARGO_PKG_VERSION"), meta.len());
        // only plain arguments cross into WSL: the steps are in wsl/start-engine.sh
        Command::new("wsl.exe")
            .args(["-d", &d, "--", "bash"])
            .arg(to_wsl(&res.join("wsl").join("start-engine.sh")))
            .arg(to_wsl(&src))
            .arg(to_wsl(res))
            .arg(to_wsl(&runs_folder()))
            .arg(PORT.to_string())
            .arg(&name)
            .creation_flags(NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Could not start WSL: {e}"))
    }
    #[cfg(not(windows))]
    {
        Command::new(res.join("bin").join("gwm"))
            .args(["serve", "--port", &PORT.to_string()])
            .env("GWM_ROOT", res)
            .env("GWM_RUNS_DIR", runs_folder())
            .spawn()
            .map_err(|e| format!("Could not start the engine: {e}"))
    }
}

/// Ask the engine to stop (it finishes what it is sending first). Runs carry on:
/// each is its own process.
fn shutdown() {
    use std::io::Write;
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
    if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
        let _ = s.write_all(b"POST /api/shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let _ = s.flush();
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Start this app's own engine. One left from an earlier start (maybe an older
/// version after an update) is stopped first.
fn ensure(app: &tauri::AppHandle) {
    let state = app.state::<Backend>();
    if answering() {
        shutdown();
        let t = Instant::now();
        while answering() && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let res = app.path().resource_dir().unwrap_or_default();
    match start(&res) {
        Ok(child) => {
            *state.child.lock().unwrap() = Some(child);
            *state.problem.lock().unwrap() = None;
            // wait up to 20 s for it to answer (WSL can take a moment to wake)
            let t = Instant::now();
            while !answering() && t.elapsed() < Duration::from_secs(20) {
                std::thread::sleep(Duration::from_millis(250));
            }
            if !answering() {
                *state.problem.lock().unwrap() = Some("The engine did not start. Check Settings, or restart the app.".into());
            }
        }
        Err(e) => *state.problem.lock().unwrap() = Some(e),
    }
}

#[tauri::command]
fn backend_status(state: State<Backend>) -> Status {
    Status {
        running: answering(),
        problem: state.problem.lock().unwrap().clone(),
        windows: cfg!(windows),
        distro: distro(),
        tray: *state.in_tray.lock().unwrap(),
    }
}

#[tauri::command]
async fn restart_backend(app: tauri::AppHandle) -> Status {
    let a = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || ensure(&a)).await;
    backend_status(app.state::<Backend>())
}

/// Open a folder in Explorer / Finder / the file manager. Engine paths from WSL
/// (`/mnt/c/...`, `/root/...`) are turned into Windows ones.
#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let win = if let Some(rest) = path.strip_prefix("/mnt/") {
            let (drive, tail) = rest.split_at(1);
            format!("{}:{}", drive.to_uppercase(), tail.replace('/', "\\"))
        } else if path.starts_with('/') {
            format!("\\\\wsl.localhost\\{}{}", distro(), path.replace('/', "\\"))
        } else {
            path
        };
        std::fs::create_dir_all(&win).ok();
        Command::new("explorer.exe").arg(&win).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        std::fs::create_dir_all(&path).ok();
        Command::new("open").arg(&path).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::fs::create_dir_all(&path).ok();
        Command::new("xdg-open").arg(&path).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

/// Windows: install WSL with Ubuntu (asks for administrator rights; a restart follows).
#[tauri::command]
fn setup_wsl() -> Result<(), String> {
    #[cfg(windows)]
    {
        let script = format!("Start-Process wsl.exe -ArgumentList '--install','-d','{}' -Verb RunAs", distro());
        Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &script])
            .creation_flags(NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    Err("only needed on Windows".into())
}

/// The script for Settings > Set up Docker, written where a terminal can run it.
#[cfg(not(windows))]
fn setup_script(name: &str, text: &str) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join("genomics-workflow-microbials");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    std::fs::write(&path, text.replace("\r\n", "\n")).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Install and start Docker in a window that shows the progress: Docker Engine
/// inside WSL (Windows) or straight on Ubuntu and Debian (Linux), Colima with
/// Homebrew (Mac; `cpus` and `memory` GB are what Colima gets).
#[tauri::command]
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
fn setup_docker(cpus: Option<u32>, memory: Option<u32>) -> Result<(), String> {
    #[cfg(windows)]
    {
        let script = "set -e; apt-get update; DEBIAN_FRONTEND=noninteractive apt-get install -y docker.io procps; \
                      (systemctl enable --now docker 2>/dev/null || service docker start); \
                      echo; echo 'Docker is ready. You can close this window and press Check again in the app.'; sleep 30";
        Command::new("cmd.exe")
            .args(["/c", "start", "Setting up Docker", "wsl.exe", "-d", &distro(), "-u", "root", "--", "bash", "-c", script])
            .creation_flags(NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let text = include_str!("setup-docker-mac.sh")
            .replace("@CPUS@", &cpus.unwrap_or(2).clamp(1, 256).to_string())
            .replace("@MEMORY@", &memory.unwrap_or(4).clamp(2, 1024).to_string());
        // Terminal opens a .command file and runs it
        let path = setup_script("Set up Docker.command", &text)?;
        Command::new("open").arg(&path).spawn().map(|_| ()).map_err(|e| format!("Terminal could not be opened: {e}"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let path = setup_script("set-up-docker.sh", include_str!("setup-docker-linux.sh"))?;
        // the terminal programs of the usual desktops, and how each takes a command
        let terminals: [(&str, &[&str]); 5] = [
            ("gnome-terminal", &["--", "bash"]),
            ("konsole", &["-e", "bash"]),
            ("xfce4-terminal", &["-x", "bash"]),
            ("x-terminal-emulator", &["-e", "bash"]),
            ("xterm", &["-e", "bash"]),
        ];
        for (program, args) in terminals {
            if Command::new(program).args(args).arg(&path).spawn().is_ok() {
                return Ok(());
            }
        }
        Err("No terminal program was found to show the progress in. Use the steps shown under the button.".into())
    }
}

// ---------------------------------------------------------------- start and stop the services
//
// "The services" are what the tools run in and what takes the memory: on
// Windows the app's Linux (WSL) with the engine and Docker inside it, on a Mac
// Colima (or Docker Desktop), on Linux Docker Engine. The side panel has a
// Start and a Stop button for them; on Windows, Quit stops them too.

/// A Mac app opened from Finder gets a bare PATH: add the folders Homebrew,
/// Docker Desktop, Colima, Rancher Desktop and MacPorts install into, so the
/// app (and the engine it starts) finds `colima` and `docker`. Same list as
/// the engine's own (`gwm-engine/src/docker.rs`).
#[cfg(not(windows))]
fn extend_path() {
    let home = dirs::home_dir().unwrap_or_default();
    let extra = [
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".docker/bin"),
        PathBuf::from("/Applications/Docker.app/Contents/Resources/bin"),
        home.join(".colima/bin"),
        home.join(".rd/bin"),
        PathBuf::from("/opt/local/bin"),
    ];
    let mut paths: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for p in extra {
        if p.is_dir() && !paths.contains(&p) {
            paths.push(p);
        }
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        std::env::set_var("PATH", joined);
    }
}

/// Runs that are going or waiting, as the engine says; nothing when it does not answer.
fn engine_runs() -> Option<usize> {
    use std::io::{Read, Write};
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    s.write_all(b"GET /api/runs HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let body = text.split_once("\r\n\r\n")?.1;
    let list: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    Some(list.as_array()?.iter().filter(|r| matches!(r["status"].as_str(), Some("running" | "queued"))).count())
}

/// A run is going: the services must stay on for it. With the engine not
/// answering, the run processes themselves are looked for (each is its own
/// `gwm exec`), so a run is never cut off by mistake.
fn run_going() -> bool {
    if let Some(n) = engine_runs() {
        return n > 0;
    }
    const RUN: &str = "gwm[^ ]* exec ";
    #[cfg(windows)]
    {
        distro_running()
            && Command::new("wsl.exe")
                .args(["-d", &distro(), "--", "pgrep", "-f", RUN])
                .creation_flags(NO_WINDOW)
                .output()
                .is_ok_and(|o| o.status.success())
    }
    #[cfg(not(windows))]
    {
        Command::new("pgrep").args(["-f", RUN]).output().is_ok_and(|o| o.status.success())
    }
}

/// The last lines a command wrote, for an error message.
#[cfg_attr(windows, allow(dead_code))]
fn last_words(out: &std::process::Output) -> String {
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines[lines.len().saturating_sub(3)..].join(" ")
}

#[cfg(target_os = "macos")]
fn has_colima() -> bool {
    Command::new("colima").arg("version").output().is_ok_and(|o| o.status.success())
}

#[cfg(target_os = "macos")]
const DOCKER_DESKTOP: &str = "/Applications/Docker.app";

/// Switch the app's Linux off (Windows): the engine and Docker go with it and
/// Windows gets the memory back.
#[cfg(windows)]
fn stop_wsl(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<Backend>();
    *state.problem.lock().unwrap() = Some("stopped".into());
    shutdown();
    if let Some(mut child) = state.child.lock().unwrap().take() {
        let _ = child.kill();
    }
    let out = Command::new("wsl.exe")
        .args(["--terminate", &distro()])
        .creation_flags(NO_WINDOW)
        .output()
        .map_err(|e| format!("Could not reach WSL: {e}"))?;
    if !out.status.success() {
        return Err(format!("{} could not be switched off.", distro()));
    }
    // With the Linux off, WSL's own machine (VmmemWSL in Task Manager) can stay
    // on and keep its memory: switch it off too, unless another Linux of the
    // user's is running in it.
    let installed = wsl_distros().unwrap_or_default();
    let others = wsl_list(&["-l", "-q", "--running"])
        .unwrap_or_default()
        .iter()
        .any(|l| installed.iter().any(|d| d.eq_ignore_ascii_case(l)));
    if !others {
        let _ = Command::new("wsl.exe").arg("--shutdown").creation_flags(NO_WINDOW).status();
    }
    Ok(())
}

fn stop_services(app: &tauri::AppHandle) -> Result<(), String> {
    if run_going() {
        return Err("A run is still going. Stop it on the Runs page, or wait until it has finished.".into());
    }
    #[cfg(windows)]
    {
        stop_wsl(app)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        if has_colima() && Command::new("colima").arg("status").output().is_ok_and(|o| o.status.success()) {
            let out = Command::new("colima").arg("stop").output().map_err(|e| e.to_string())?;
            return if out.status.success() { Ok(()) } else { Err(format!("Colima could not be stopped: {}", last_words(&out))) };
        }
        if Path::new(DOCKER_DESKTOP).exists() {
            let out = Command::new("osascript").args(["-e", "quit app \"Docker\""]).output().map_err(|e| e.to_string())?;
            return if out.status.success() { Ok(()) } else { Err(format!("Docker Desktop could not be closed: {}", last_words(&out))) };
        }
        Err("Docker is not started by Colima or Docker Desktop here, so the app cannot stop it.".into())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = app;
        // the system asks for the password in its own window
        let out = Command::new("systemctl").args(["stop", "docker.socket", "docker.service"]).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("Docker could not be stopped: {} In a terminal: sudo systemctl stop docker.socket docker.service", last_words(&out)))
        }
    }
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
fn start_services(app: &tauri::AppHandle, cpus: Option<u32>, memory: Option<u32>) -> Result<(), String> {
    #[cfg(windows)]
    {
        if !answering() {
            ensure(app);
        }
        let state = app.state::<Backend>();
        if let Some(problem) = state.problem.lock().unwrap().clone() {
            return Err(match problem.as_str() {
                "wsl" => "WSL is not installed yet.".into(),
                "distro" => format!("{} is not installed yet.", distro()),
                _ => problem,
            });
        }
        let out = Command::new("wsl.exe")
            .args(["-d", &distro(), "-u", "root", "--", "sh", "-c", "command -v docker >/dev/null 2>&1 || exit 7; systemctl start docker 2>/dev/null || service docker start"])
            .creation_flags(NO_WINDOW)
            .output()
            .map_err(|e| format!("Could not reach WSL: {e}"))?;
        match out.status.code() {
            Some(0) => Ok(()),
            Some(7) => Err("Docker is not installed yet. Open Settings and press Set up Docker.".into()),
            _ => Err("Docker could not be started inside WSL. Open Settings and press Check again.".into()),
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        if has_colima() {
            let mut cmd = Command::new("colima");
            cmd.arg("start");
            // every start sets the machine's size: all the Mac's memory but 4 GB,
            // also for a machine made earlier with less
            if let (Some(c), Some(m)) = (cpus, memory) {
                cmd.args(["--cpu", &c.clamp(1, 256).to_string()]);
                cmd.args(["--memory", &m.clamp(2, 1024).to_string()]);
            }
            let out = cmd.output().map_err(|e| e.to_string())?;
            return if out.status.success() { Ok(()) } else { Err(format!("Colima could not start: {}", last_words(&out))) };
        }
        if Path::new(DOCKER_DESKTOP).exists() {
            // it takes a minute to be ready; the app keeps looking
            return Command::new("open").args(["-a", "Docker"]).spawn().map(|_| ()).map_err(|e| format!("Docker Desktop could not be opened: {e}"));
        }
        Err("Docker is not installed yet. Open Settings and press Set up Docker.".into())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = app;
        // the system asks for the password in its own window
        let out = Command::new("systemctl").args(["start", "docker"]).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("Docker could not be started: {} In a terminal: sudo systemctl start docker", last_words(&out)))
        }
    }
}

/// Side panel > Start: switch on what the tools run in (`cpus` and `memory`
/// GB size Colima on its very first start).
#[tauri::command]
async fn services_start(app: tauri::AppHandle, cpus: Option<u32>, memory: Option<u32>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || start_services(&app, cpus, memory)).await.map_err(|e| e.to_string())?
}

/// Side panel > Stop: switch it off, so it takes no memory. Refused while a run is going.
#[tauri::command]
async fn services_stop(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || stop_services(&app)).await.map_err(|e| e.to_string())?
}

/// Open a web page or a new email in the system's browser or mail program.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("mailto:")) {
        return Err("only web and email links can be opened".into());
    }
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler").creation_flags(NO_WINDOW);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = Command::new("xdg-open");
    cmd.arg(&url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Ask where to save a text file (a run's problem report) and write it there.
/// Returns where it went, or nothing when the dialog was closed.
#[tauri::command]
async fn save_text_file(app: tauri::AppHandle, name: String, text: String) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_file_name(&name)
            .add_filter("Text file", &["txt"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    std::fs::write(&path, text).map_err(|e| format!("Could not save to {}: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

// ---------------------------------------------------------------- remove everything
// Settings > Remove everything, the parts only the desktop app can reach on
// Windows: Docker inside WSL and the WSL Ubuntu itself. (The tool containers
// and the data folder are the engine's: `/api/cleanup`.)

#[derive(Serialize, Default)]
struct Extras {
    /// Docker and Ubuntu can be removed here (Windows with the app's WSL distribution)
    windows: bool,
    distro: String,
    /// Docker's own folder inside WSL; nothing when Docker is not installed there
    docker_bytes: Option<u64>,
    /// the Ubuntu's disk file
    ubuntu_bytes: Option<u64>,
}

/// A command as root inside the app's WSL distribution: its output, or what went wrong.
#[cfg(windows)]
fn wsl_root(script: &str) -> Result<String, String> {
    let out = Command::new("wsl.exe")
        .args(["-d", &distro(), "-u", "root", "--", "bash", "-c", script])
        .creation_flags(NO_WINDOW)
        .output()
        .map_err(|e| format!("Could not start WSL: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it did not work").trim().to_string())
    }
}

/// Size of the Ubuntu's disk file (Windows keeps where it is in the registry).
#[cfg(windows)]
fn ubuntu_bytes(distro: &str) -> Option<u64> {
    if distro.is_empty() || !distro.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        return None;
    }
    let script = format!(
        "Get-ChildItem 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Lxss' | ForEach-Object {{ Get-ItemProperty $_.PSPath }} | \
         Where-Object {{ $_.DistributionName -eq '{distro}' }} | ForEach-Object {{ (Get-Item -LiteralPath (Join-Path $_.BasePath 'ext4.vhdx')).Length }}"
    );
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(NO_WINDOW)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| l.trim().parse().ok())
}

#[tauri::command]
async fn extras_overview() -> Extras {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(|| {
            let d = distro();
            if !wsl_distros().is_some_and(|l| l.iter().any(|x| x.eq_ignore_ascii_case(&d))) {
                return Extras { distro: d, ..Extras::default() };
            }
            // newer Docker keeps its images in containerd's folder
            let docker = wsl_root("command -v docker >/dev/null 2>&1 && du -sbc /var/lib/docker /var/lib/containerd 2>/dev/null | tail -1 | cut -f1");
            Extras {
                windows: true,
                docker_bytes: docker.ok().and_then(|t| t.trim().parse().ok()),
                ubuntu_bytes: ubuntu_bytes(&d),
                distro: d,
            }
        })
        .await
        .unwrap_or_default()
    }
    #[cfg(not(windows))]
    Extras::default()
}

/// Windows: take Docker out of the app's Ubuntu, with every container in it.
#[tauri::command]
async fn remove_docker() -> Result<(), String> {
    #[cfg(windows)]
    {
        let script = "(systemctl disable --now docker.socket docker containerd 2>/dev/null || service docker stop 2>/dev/null || true); \
                      for p in docker.io docker-ce docker-ce-cli containerd containerd.io; do \
                        if dpkg -s \"$p\" >/dev/null 2>&1; then DEBIAN_FRONTEND=noninteractive apt-get purge -y \"$p\" >/dev/null || exit 1; fi; \
                      done; \
                      rm -rf /var/lib/docker /var/lib/containerd; \
                      if command -v docker >/dev/null 2>&1; then echo 'Docker is still there: it was not installed the usual way' >&2; exit 1; fi";
        tauri::async_runtime::spawn_blocking(move || wsl_root(script).map(|_| ()))
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    Err("only on Windows".into())
}

/// Windows: delete the app's WSL Ubuntu, with everything kept in it (the tool
/// containers, Docker, the data folder and anything else of the user's there).
#[tauri::command]
async fn remove_ubuntu(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            // our engine lives there: stop it first
            shutdown();
            let state = app.state::<Backend>();
            if let Some(mut child) = state.child.lock().unwrap().take() {
                let _ = child.kill();
            }
            let d = distro();
            let out = Command::new("wsl.exe")
                .args(["--unregister", &d])
                .creation_flags(NO_WINDOW)
                .output()
                .map_err(|e| format!("Could not start WSL: {e}"))?;
            if !out.status.success() {
                return Err(format!("Windows could not remove {d}. Try again after restarting the computer."));
            }
            *state.problem.lock().unwrap() = Some("distro".into());
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("only on Windows".into())
    }
}

/// Close the app fully (also from the system tray).
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// How far the update's download is (the GUI asks while `update_install` works).
#[tauri::command]
fn update_progress(state: State<Backend>) -> UpdateProgress {
    UpdateProgress { done: state.update_done.load(Ordering::Relaxed), total: state.update_total.load(Ordering::Relaxed) }
}

/// What went wrong with an update, in plain words.
fn update_error(e: tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error as E;
    match e {
        E::ReleaseNotFound | E::TargetNotFound(_) | E::TargetsNotFound(_) => {
            "The newest version has no automatic update for this computer. Use the download page instead.".into()
        }
        E::Reqwest(_) | E::Network(_) => "Could not reach the download. Check the internet connection and try again.".into(),
        E::Minisign(_) | E::SignedVersionMismatch { .. } | E::MissingSignedVersion => {
            "The downloaded update did not pass its safety check, so it was not installed. Try again, or use the download page.".into()
        }
        E::AuthenticationFailed => "The update needs your password to install, and that was cancelled.".into(),
        other => format!("The update could not be installed: {other}. Use the download page instead."),
    }
}

/// Update the app in place: download the newest version for this computer, check
/// it carries our signature, install it over this one and open it again. Windows:
/// the installer closes this app and starts the new one. Runs carry on: each is
/// its own process, using its own copy of the engine.
#[tauri::command]
async fn update_install(app: tauri::AppHandle) -> Result<(), String> {
    if app.state::<Backend>().updating.swap(true, Ordering::SeqCst) {
        return Err("An update is already being installed.".into());
    }
    let result = update_now(&app).await;
    app.state::<Backend>().updating.store(false, Ordering::SeqCst);
    result?;
    // Mac and Linux: the new version is in place, start it
    shutdown();
    app.restart()
}

async fn update_now(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    {
        let state = app.state::<Backend>();
        state.update_done.store(0, Ordering::Relaxed);
        state.update_total.store(0, Ordering::Relaxed);
    }
    let closing = app.clone();
    let updater = app
        .updater_builder()
        // Windows, right before the installer takes over: stop our engine so its files can be replaced
        .on_before_exit(move || {
            shutdown();
            closing.cleanup_before_exit();
        })
        .build()
        .map_err(update_error)?;
    let Some(update) = updater.check().await.map_err(update_error)? else {
        return Err("You already have the newest version.".into());
    };
    let counting = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                let state = counting.state::<Backend>();
                state.update_done.fetch_add(chunk as u64, Ordering::Relaxed);
                if let Some(total) = total {
                    state.update_total.store(total, Ordering::Relaxed);
                }
            },
            || {},
        )
        .await
        .map_err(update_error)
}

/// Bring the window back (from the system tray, or when the app is started again).
fn show_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// The system tray icon: click to open the window, right-click for Open and Quit.
#[cfg(not(target_os = "linux"))]
fn tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open = MenuItem::with_id(app, "open", "Open Genomics Workflow Microbials", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Genomics Workflow Microbials")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn main() {
    #[cfg(not(windows))]
    extend_path();
    let app = tauri::Builder::default()
        // started a second time: show the window that is already there
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Backend::default())
        .invoke_handler(tauri::generate_handler![
            backend_status,
            restart_backend,
            open_folder,
            open_url,
            save_text_file,
            setup_wsl,
            setup_docker,
            services_start,
            services_stop,
            update_install,
            update_progress,
            extras_overview,
            remove_docker,
            remove_ubuntu,
            quit_app
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            #[cfg(not(target_os = "linux"))]
            let in_tray = tray(&handle).is_ok();
            #[cfg(target_os = "linux")]
            let in_tray = false;
            *app.state::<Backend>().in_tray.lock().unwrap() = in_tray;
            std::thread::spawn(move || ensure(&handle));
            Ok(())
        })
        .on_window_event(|window, event| {
            // with a tray icon, closing the window keeps the app there (Quit is in the icon's menu)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if *window.state::<Backend>().in_tray.lock().unwrap() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("starting Genomics Workflow Microbials");
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            // Windows: quitting also switches the app's Linux off, so it stops taking
            // memory. Not while a run is going (runs outlive the app), not when an
            // update is about to open the app again, not when it is off already.
            #[cfg(windows)]
            {
                let state = app.state::<Backend>();
                if !state.updating.load(Ordering::Relaxed) && distro_running() && !run_going() {
                    let _ = stop_wsl(app);
                    return;
                }
            }
            // the window's own engine stops with it; runs carry on in their own processes
            shutdown();
            if let Some(mut child) = app.state::<Backend>().child.lock().unwrap().take() {
                let _ = child.kill();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_for_wsl() {
        assert_eq!(to_wsl(Path::new(r"C:\Program Files\GWM")), "/mnt/c/Program Files/GWM");
        assert_eq!(to_wsl(Path::new(r"\\?\D:\Apps")), "/mnt/d/Apps");
    }
}
