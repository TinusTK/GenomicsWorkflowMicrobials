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
    let out = Command::new("wsl.exe").args(["-l", "-q"]).creation_flags(NO_WINDOW).output().ok()?;
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

/// Install and start Docker Engine inside WSL, in a window that shows the progress.
#[tauri::command]
fn setup_docker() -> Result<(), String> {
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
    #[cfg(not(windows))]
    Err("Install Docker Engine (Linux) or Colima (Mac), then press Check again.".into())
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
    let app = tauri::Builder::default()
        // started a second time: show the window that is already there
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app)))
        .manage(Backend::default())
        .invoke_handler(tauri::generate_handler![backend_status, restart_backend, open_folder, open_url, setup_wsl, setup_docker])
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
