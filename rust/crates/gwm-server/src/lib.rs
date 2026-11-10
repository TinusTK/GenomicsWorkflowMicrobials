//! The app's local JSON API: the GUI (desktop window or a browser) talks to it.
//! Runs are started as their own `gwm exec <run>` process, so closing the
//! window or restarting the app never stops a run.

pub mod hardware;
pub mod jobs;

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use gwm_core::paths::native_path;
use gwm_core::reads::{find_samples, natural_cmp, pairing_problems, Pairing};
use gwm_core::{compiler, Catalog, Workflow};
use gwm_engine::runs::{self, StartOptions};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct App {
    cat: Catalog,
    /// the program that runs a run (`<exe> exec <run_id>`)
    exe: PathBuf,
}

/// An error the GUI shows as a message.
struct ApiError(StatusCode, String);

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        let e = e.into();
        let msg = format!("{e:#}");
        let code = if msg.starts_with("unknown") || msg.starts_with("no file") || msg.contains("No such file") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        };
        ApiError(code, msg)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

type Api<T = Value> = Result<Json<T>, ApiError>;

fn ok(v: impl serde::Serialize) -> Api {
    Ok(Json(serde_json::to_value(v)?))
}

/// Serve the API and (when `gui` is given) the built GUI from that folder.
pub async fn serve(addr: SocketAddr, cat: Catalog, gui: Option<PathBuf>) -> anyhow::Result<()> {
    let removed = runs::clean_orphan_work(&cat);
    if removed > 0 {
        eprintln!("Removed the working files of {removed} deleted run(s)");
    }
    eprintln!("Run folders: {}", cat.runs.display());
    let app = App {
        cat,
        exe: std::env::current_exe()?,
    };
    // the desktop app stops its engine when it closes (runs carry on: they are their own processes)
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    let stopper = stop.clone();
    let api = Router::new()
        .route(
            "/api/health",
            get(|| async { Json(json!({"ok": true, "version": env!("CARGO_PKG_VERSION")})) }),
        )
        .route(
            "/api/shutdown",
            post(move || async move {
                stopper.notify_one();
                Json(json!({"ok": true}))
            }),
        )
        .route("/api/hardware", get(hardware_check))
        .route("/api/usage", get(usage))
        .route("/api/workflows", get(list_workflows))
        .route("/api/workflows/save", post(save_workflow))
        .route("/api/workflows/check", post(check_workflow))
        .route("/api/workflows/{id}", get(get_workflow).delete(delete_workflow))
        .route("/api/tools", get(list_tools))
        .route("/api/databases", get(list_databases))
        .route("/api/databases/{id}/install", post(install_database))
        .route("/api/jobs/{id}", get(get_job))
        .route("/api/browse", get(browse))
        .route("/api/reads", post(pair_reads))
        .route("/api/runs", get(list_runs).post(start_run))
        .route("/api/runs/{id}", get(run_detail).delete(delete_run))
        .route("/api/runs/{id}/stop", post(stop_run))
        .route("/api/runs/{id}/resume", post(resume_run))
        .route("/api/runs/{id}/file", get(run_file))
        .with_state(app)
        // the desktop window loads the GUI from its own origin
        .layer(CorsLayer::permissive());
    let router = match gui {
        Some(dir) => api.fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html")))),
        None => api,
    };
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("Genomics Workflow Microbials: http://{addr}/");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = stop.notified() => {}
            }
        })
        .await?;
    Ok(())
}

// ---------------------------------------------------------------- computer

async fn hardware_check(State(a): State<App>) -> Api {
    ok(tokio::task::spawn_blocking(move || hardware::check(&a.cat)).await?)
}

async fn usage(State(a): State<App>) -> Api {
    ok(hardware::usage(&a.cat))
}

// ---------------------------------------------------------------- workflows

async fn list_workflows(State(a): State<App>) -> Api {
    ok(a.cat.list_workflows()?)
}

#[derive(Deserialize)]
struct VersionQ {
    version: Option<u32>,
}

/// The workflow with its versions, run order, which steps will run and any problem.
async fn get_workflow(State(a): State<App>, UrlPath(id): UrlPath<String>, Query(q): Query<VersionQ>) -> Api {
    let wf = a.cat.load_workflow(&id, q.version)?;
    let tools = a.cat.load_tools()?;
    let versions: Vec<u32> = if wf.builtin {
        vec![wf.saved_version]
    } else {
        a.cat.user_workflow_versions(&id).into_iter().map(|(v, _)| v).collect()
    };
    let (order, runnable, error) = match compiler::resolve(&wf, &tools) {
        Ok(r) => (
            r.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
            compiler::runnable(&wf, &tools).ok(),
            None,
        ),
        Err(e) => (vec![], None, Some(e.to_string())),
    };
    ok(json!({
        "workflow": wf, "builtin": wf.builtin, "version": wf.saved_version, "versions": versions,
        "order": order, "runnable": runnable, "error": error,
    }))
}

#[derive(Deserialize)]
struct SaveReq {
    workflow: Workflow,
    new_id: Option<String>,
}

async fn save_workflow(State(a): State<App>, Json(b): Json<SaveReq>) -> Api {
    // never save a workflow that can't run
    compiler::resolve(&b.workflow, &a.cat.load_tools()?)?;
    let version = a.cat.save_workflow(&b.workflow, b.new_id.as_deref())?;
    ok(json!({"id": b.new_id.unwrap_or(b.workflow.id), "version": version}))
}

async fn delete_workflow(State(a): State<App>, UrlPath(id): UrlPath<String>) -> Api {
    a.cat.delete_workflow(&id)?;
    ok(json!({"ok": true}))
}

#[derive(Deserialize)]
struct CheckReq {
    workflow: Workflow,
    input: Option<String>,
    #[serde(default)]
    steps_off: Vec<String>,
    pairing: Option<Pairing>,
}

/// Plain warnings for running this workflow on these reads, here.
async fn check_workflow(State(a): State<App>, Json(b): Json<CheckReq>) -> Api {
    let mut wf = b.workflow;
    let tools = a.cat.load_tools()?;
    let error = compiler::resolve(&wf, &tools).err().map(|e| e.to_string());
    for s in &b.steps_off {
        if let Some(st) = wf.steps.get_mut(s) {
            st.enabled = Some(false);
        }
    }
    let found = b
        .input
        .as_deref()
        .map(|i| find_samples(&native_path(i), b.pairing.as_ref()));
    let cat = a.cat.clone();
    let wf2 = wf.clone();
    let n = found.as_ref().map(|f| f.samples.len());
    let mut warns = tokio::task::spawn_blocking(move || {
        let hw = hardware::check(&cat);
        hardware::warnings(&cat, &wf2, &hw, n)
    })
    .await?;
    if let Some(e) = error {
        warns.insert(0, json!({"level": "error", "step": null, "message": e}));
    }
    if let Some(f) = &found {
        if f.samples.is_empty() {
            warns.insert(0, json!({"level": "error", "step": null, "message": format!("No paired reads found (pairing on {} / {}).", f.r1, f.r2)}));
        }
        for m in pairing_problems(f) {
            warns.push(json!({"level": "error", "step": null, "message": m}));
        }
    }
    let runnable = compiler::runnable(&wf, &tools).ok();
    ok(json!({"warnings": warns, "pairing": found, "runnable": runnable}))
}

// ---------------------------------------------------------------- tools and databases

async fn list_tools(State(a): State<App>) -> Api {
    let tools = a.cat.load_tools()?;
    let out: Vec<Value> = tools
        .values()
        .map(|t| {
            let mut v = serde_json::to_value(t).unwrap_or_default();
            v["source"] = json!(t.source);
            v["variants"] = json!(t.variants.keys().collect::<Vec<_>>());
            v
        })
        .collect();
    ok(out)
}

async fn list_databases(State(a): State<App>) -> Api {
    ok(a.cat.load_databases()?.into_values().collect::<Vec<_>>())
}

async fn install_database(State(a): State<App>, UrlPath(id): UrlPath<String>) -> Api {
    let name = a
        .cat
        .load_databases()?
        .get(&id)
        .map(|d| d.name.clone())
        .ok_or_else(|| anyhow::anyhow!("unknown database {id}"))?;
    let cat = a.cat.clone();
    ok(jobs::spawn(&format!("Install {name}"), move |r| {
        jobs::install_database(&cat, &id, r)
    }))
}

async fn get_job(UrlPath(id): UrlPath<String>) -> Api {
    ok(jobs::get(&id).ok_or_else(|| anyhow::anyhow!("unknown job {id}"))?)
}

// ---------------------------------------------------------------- reads

#[derive(Deserialize)]
struct BrowseQ {
    path: Option<String>,
}

/// Folders and read files in a folder (the reads picker).
async fn browse(State(a): State<App>, Query(q): Query<BrowseQ>) -> Api {
    // start in Documents (on Windows the Windows one), where people keep things
    let documents = a.cat.runs.parent().filter(|p| p.is_dir()).map(Path::to_path_buf);
    let path = q
        .path
        .filter(|p| !p.trim().is_empty())
        .map(|p| native_path(p.trim()))
        .or(documents)
        .or_else(gwm_core::paths::user_home_dir)
        .unwrap_or_else(|| PathBuf::from("/"));
    let path = fs::canonicalize(&path).unwrap_or(path);
    let mut dirs = Vec::new();
    let mut reads = Vec::new();
    if let Ok(rd) = fs::read_dir(&path) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                dirs.push(name);
            } else if name.ends_with(".gz") || name.ends_with(".fastq") || name.ends_with(".fq") {
                reads.push(name);
            }
        }
    }
    dirs.sort_by(|a, b| natural_cmp(a, b));
    reads.sort_by(|a, b| natural_cmp(a, b));
    reads.truncate(200);
    ok(json!({"path": path, "parent": path.parent(), "dirs": dirs, "reads": reads}))
}

#[derive(Deserialize)]
struct ReadsReq {
    input: String,
    pairing: Option<Pairing>,
}

async fn pair_reads(Json(b): Json<ReadsReq>) -> Api {
    let found = find_samples(&native_path(&b.input), b.pairing.as_ref());
    let problems = pairing_problems(&found);
    ok(json!({"found": found, "problems": problems}))
}

// ---------------------------------------------------------------- runs

async fn list_runs(State(a): State<App>) -> Api {
    runs::clean_orphan_work(&a.cat);
    let mut list = runs::list(&a.cat);
    for r in &mut list {
        if r["status"] == "running" {
            if let Some(id) = r["id"].as_str() {
                if let Ok(fresh) = runs::refresh(&a.cat, id) {
                    r["status"] = fresh["status"].clone();
                }
            }
        }
    }
    ok(list)
}

#[derive(Deserialize)]
struct StartReq {
    workflow: String,
    input: String,
    #[serde(flatten)]
    opts: StartOptions,
}

/// The app's settings a run needs (data folder, runs folder, ...), passed on to it.
fn run_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| k.starts_with("GWM_") || k.starts_with("NTG_") || k == "PATH" || k == "HOME")
        .collect()
}

/// Start `gwm exec <run>` so that it outlives the app. With systemd (Linux, and
/// WSL with systemd on), as its own service: on Windows, closing the app ends its
/// WSL session, which would otherwise take the run with it.
fn spawn_exec(a: &App, run_id: &str) -> anyhow::Result<()> {
    let dir = runs::run_path(&a.cat, run_id)?;
    let log_path = dir.join(runs::DETAILS).join("exec.log");
    #[cfg(target_os = "linux")]
    if Path::new("/run/systemd/system").exists() {
        // SAFETY: getuid has no preconditions
        let root = unsafe { libc::getuid() } == 0;
        let mut cmd = Command::new("systemd-run");
        if !root {
            cmd.arg("--user");
        }
        let unit: String = run_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        // a new name each time: a resumed run never clashes with its earlier service
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        cmd.args(["--quiet", "--collect", &format!("--unit=gwm-run-{unit}-{secs}")])
            .arg(format!("--property=StandardOutput=append:{}", log_path.display()))
            .arg(format!("--property=StandardError=append:{}", log_path.display()))
            .arg(format!("--working-directory={}", dir.display()));
        for (k, v) in run_env() {
            cmd.arg(format!("--setenv={k}={v}"));
        }
        cmd.arg(&a.exe)
            .args(["exec", run_id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Ok(out) = cmd.output() {
            if out.status.success() {
                return Ok(());
            }
            eprintln!("systemd-run: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
    }
    let log = fs::File::create(&log_path)?;
    let mut cmd = Command::new(&a.exe);
    cmd.args(["exec", run_id])
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // its own session: not stopped when the app (or its WSL window) closes
        // SAFETY: setsid is async-signal-safe and touches no memory
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    cmd.spawn()?;
    Ok(())
}

async fn start_run(State(a): State<App>, Json(b): Json<StartReq>) -> Api {
    let backend = gwm_engine::backend::detect();
    let run = runs::prepare(&a.cat, &b.workflow, &native_path(&b.input), &b.opts, backend.name())?;
    spawn_exec(&a, run["id"].as_str().unwrap_or_default())?;
    ok(run)
}

async fn run_detail(State(a): State<App>, UrlPath(id): UrlPath<String>) -> Api {
    ok(runs::detail(&a.cat, &id)?)
}

async fn stop_run(State(a): State<App>, UrlPath(id): UrlPath<String>) -> Api {
    ok(json!({"stopping": runs::request_stop(&a.cat, &id)?}))
}

async fn resume_run(State(a): State<App>, UrlPath(id): UrlPath<String>) -> Api {
    let run = runs::refresh(&a.cat, &id)?;
    if run["status"] == "running" {
        return Err(ApiError(StatusCode::CONFLICT, "this run is still running".into()));
    }
    spawn_exec(&a, &id)?;
    ok(json!({"ok": true}))
}

async fn delete_run(State(a): State<App>, UrlPath(id): UrlPath<String>) -> Api {
    let run = runs::refresh(&a.cat, &id)?;
    if run["status"] == "running" {
        return Err(ApiError(StatusCode::CONFLICT, "stop the run before deleting it".into()));
    }
    runs::delete(&a.cat, &id)?;
    ok(json!({"ok": true}))
}

#[derive(Deserialize)]
struct FileQ {
    path: String,
    download: Option<u8>,
}

async fn run_file(
    State(a): State<App>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<FileQ>,
) -> Result<Response, ApiError> {
    let f = runs::result_file(&a.cat, &id, &q.path)?;
    let data = tokio::fs::read(&f).await?;
    let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let ctype = match f.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "json" => "application/json",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "gz" => "application/gzip",
        _ => "text/plain; charset=utf-8",
    };
    let mut resp = (StatusCode::OK, [(header::CONTENT_TYPE, ctype.to_string())], data).into_response();
    if q.download == Some(1) {
        resp.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\"").parse().unwrap(),
        );
    }
    Ok(resp)
}
