//! The app's local JSON API: the GUI (desktop window or a browser) talks to it.
//! Runs are started as their own `gwm exec <run>` process, so closing the
//! window or restarting the app never stops a run.

pub mod autosetup;
pub mod build;
pub mod cleanup;
pub mod hardware;
pub mod import;
pub mod jobs;
pub mod mlstdb;
mod selftest;
pub mod tooledit;
pub mod toolreport;

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
use gwm_core::reads::{find_input, input_problems, is_fasta, natural_cmp, Pairing, FASTA_MODE};
use gwm_core::{compiler, galaxy, Catalog, Workflow};
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
        .route("/api/workflows/import", post(import_workflow))
        .route("/api/workflows/{id}", get(get_workflow).delete(delete_workflow))
        .route("/api/tools", get(list_tools))
        .route("/api/tools/add", post(add_tool))
        .route("/api/tools/updates", get(tool_updates))
        .route("/api/tools/{id}", axum::routing::delete(delete_tool))
        .route("/api/tools/{id}/outputs", post(set_tool_outputs))
        .route("/api/tools/{id}/definition", post(save_tool))
        .route("/api/tools/{id}/help", post(tool_help))
        .route("/api/tools/{id}/try", post(try_tool))
        .route("/api/tools/{id}/setup", post(set_up_tool))
        .route("/api/tools/{id}/build", post(build_tool))
        .route("/api/tools/{id}/report", get(tool_report))
        .route("/api/tools/{id}/versions", get(tool_versions).post(add_tool_version))
        .route("/api/update", get(app_update))
        .route("/api/cleanup", get(cleanup_overview).post(cleanup_remove))
        .route("/api/databases", get(list_databases))
        .route("/api/databases/{id}/install", post(install_database))
        .route("/api/mlst-database", get(mlst_status))
        .route("/api/mlst-database/use", post(mlst_use))
        .route("/api/mlst-database/key", post(mlst_key))
        .route("/api/mlst-database/oauth/start", post(mlst_oauth_start))
        .route("/api/mlst-database/oauth/finish", post(mlst_oauth_finish))
        .route("/api/mlst-database/update", post(mlst_update))
        .route("/api/mlst-database/check", post(mlst_check))
        .route("/api/mlst-database/remove", post(mlst_remove))
        .route("/api/jobs/{id}", get(get_job))
        .route("/api/browse", get(browse))
        .route("/api/reads", post(pair_reads))
        .route("/api/runs", get(list_runs).post(start_run))
        .route("/api/runs/{id}", get(run_detail).delete(delete_run))
        .route("/api/runs/{id}/stop", post(stop_run))
        .route("/api/runs/{id}/resume", post(resume_run))
        .route("/api/runs/{id}/file", get(run_file))
        .route("/api/runs/{id}/report", get(run_report))
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
        .map(|i| find_input(&native_path(i), wf.input_type(), b.pairing.as_ref()));
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
            let message = if f.mode == FASTA_MODE {
                "No FASTA files found in this folder (.fasta, .fa, .fna, .fas, .ffn, also with .gz).".to_string()
            } else {
                format!("No paired reads found (pairing on {} / {}).", f.r1, f.r2)
            };
            warns.insert(0, json!({"level": "error", "step": null, "message": message}));
        }
        for m in input_problems(f) {
            warns.push(json!({"level": "error", "step": null, "message": m}));
        }
    }
    let runnable = compiler::runnable(&wf, &tools).ok();
    ok(json!({"warnings": warns, "pairing": found, "runnable": runnable}))
}

#[derive(Deserialize)]
struct ImportReq {
    /// the text of a Galaxy workflow (.ga) file
    ga: String,
    /// false: only say what the import would do
    #[serde(default)]
    apply: bool,
}

/// Import a Galaxy workflow: first the list of what each step becomes, then
/// (with `apply`) add the tool versions and tools it needs and save it.
async fn import_workflow(State(a): State<App>, Json(b): Json<ImportReq>) -> Api {
    let cat = a.cat.clone();
    let out = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let tools = cat.load_tools()?;
        let builtin: Vec<String> = cat.builtin_workflows()?.into_iter().map(|w| w.id).collect();
        let taken = |id: &str| builtin.iter().any(|b| b == id) || !cat.user_workflow_versions(id).is_empty();
        let imp = galaxy::analyze(&b.ga, &tools, &taken)?;
        if !b.apply {
            return Ok(json!({
                "id": imp.workflow.id, "name": imp.workflow.name, "steps": imp.steps,
                "new_versions": imp.new_versions,
                "new_tools": imp.new_tools.iter().map(|t| &t.id).collect::<Vec<_>>(),
            }));
        }
        let name = imp.workflow.name.clone();
        let done = galaxy::apply(&cat, imp, &import::Online)?;
        Ok(json!({"id": done.id, "name": name, "version": done.version, "steps": done.steps}))
    })
    .await??;
    ok(out)
}

// ---------------------------------------------------------------- tools and databases

async fn list_tools(State(a): State<App>) -> Api {
    let tools = a.cat.load_tools()?;
    let hidden = import::hidden_outputs(&a.cat);
    let out: Vec<Value> = tools
        .values()
        .map(|t| {
            let mut v = serde_json::to_value(t).unwrap_or_default();
            v["source"] = json!(t.source);
            v["variants"] = json!(t.variants.keys().collect::<Vec<_>>());
            v["folder"] = json!(t.dir);
            v["hidden_outputs"] = json!(hidden.get(&t.id).cloned().unwrap_or_default());
            // it writes reports, which the app also offers as tables (its `tables` output)
            v["report_tables"] = json!(!t.report_outputs.is_empty());
            v
        })
        .collect();
    ok(out)
}

#[derive(Deserialize)]
struct OutputsReq {
    /// outputs not shown on the Run page
    hidden: Vec<String>,
}

/// Which of a tool's outputs the Run page shows (all of them unless switched off).
async fn set_tool_outputs(State(a): State<App>, UrlPath(id): UrlPath<String>, Json(b): Json<OutputsReq>) -> Api {
    let tools = a.cat.load_tools()?;
    let tool = tools.get(&id).ok_or_else(|| anyhow::anyhow!("unknown tool {id}"))?;
    let hidden: Vec<String> = b
        .hidden
        .into_iter()
        .filter(|o| tool.outputs.contains_key(o) || o == import::REPORT_TABLES)
        .collect();
    import::set_hidden_outputs(&a.cat, &id, hidden.clone())?;
    ok(json!({"hidden_outputs": hidden}))
}

#[derive(Deserialize)]
struct AddToolReq {
    /// the tool's GitHub link
    url: String,
    /// add it even though the app has this tool built in (the user was told)
    #[serde(default)]
    anyway: bool,
}

/// Add a tool from its GitHub repository (a draft to finish in the editor,
/// unless the repository carries its own definition).
async fn add_tool(State(a): State<App>, Json(b): Json<AddToolReq>) -> Api {
    ok(tokio::task::spawn_blocking(move || tooledit::add_from_github(&a.cat, &b.url, b.anyway)).await??)
}

async fn save_tool(State(a): State<App>, UrlPath(id): UrlPath<String>, Json(b): Json<tooledit::Definition>) -> Api {
    tooledit::save_definition(&a.cat, &id, b)?;
    ok(json!({"ok": true}))
}

async fn delete_tool(State(a): State<App>, UrlPath(id): UrlPath<String>) -> Api {
    tooledit::delete_tool(&a.cat, &id)?;
    toolreport::forget(&a.cat, &id);
    ok(json!({"ok": true}))
}

#[derive(Deserialize)]
struct HelpReq {
    /// the program to ask, as typed in a terminal
    program: String,
}

async fn tool_help(State(a): State<App>, UrlPath(id): UrlPath<String>, Json(b): Json<HelpReq>) -> Api {
    ok(tool_job(
        &a.cat,
        id,
        "help",
        "Show the tool's help",
        move |cat, id, r| tooledit::show_help(cat, id, &b.program, r),
    ))
}

async fn try_tool(State(a): State<App>, UrlPath(id): UrlPath<String>, Json(b): Json<tooledit::TryRequest>) -> Api {
    ok(tool_job(&a.cat, id, "try", "Try on one sample", move |cat, id, r| {
        tooledit::try_tool(cat, id, b, r)
    }))
}

async fn tool_versions(State(a): State<App>, UrlPath(id): UrlPath<String>) -> Api {
    ok(tokio::task::spawn_blocking(move || tooledit::versions(&a.cat, &id)).await??)
}

#[derive(Deserialize)]
struct AddVersionReq {
    version: String,
}

async fn add_tool_version(State(a): State<App>, UrlPath(id): UrlPath<String>, Json(b): Json<AddVersionReq>) -> Api {
    tokio::task::spawn_blocking(move || tooledit::add_version(&a.cat, &id, &b.version)).await??;
    ok(json!({"ok": true}))
}

/// Tools with a newer version than any on this computer.
async fn tool_updates(State(a): State<App>) -> Api {
    ok(tokio::task::spawn_blocking(move || tooledit::newer_versions(&a.cat)).await??)
}

/// Is there a newer release of the app?
async fn app_update() -> Api {
    ok(tokio::task::spawn_blocking(tooledit::app_update).await??)
}

/// What "Remove everything" could remove, with sizes.
async fn cleanup_overview(State(a): State<App>) -> Api {
    ok(tokio::task::spawn_blocking(move || cleanup::overview(&a.cat)).await?)
}

#[derive(Deserialize)]
struct CleanupReq {
    #[serde(default)]
    containers: bool,
    #[serde(default)]
    data: bool,
}

/// Remove the tool containers and/or the data folder (never the run folders).
async fn cleanup_remove(State(a): State<App>, Json(b): Json<CleanupReq>) -> Api {
    ok(tokio::task::spawn_blocking(move || cleanup::remove(&a.cat, b.containers, b.data)).await??)
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

// ---------------------------------------------------------------- MLST typing's own database

/// Where the user's own MLST database stands (never the key itself).
async fn mlst_status(State(a): State<App>) -> Api {
    ok(tokio::task::spawn_blocking(move || mlstdb::status(&a.cat)).await??)
}

#[derive(Deserialize)]
struct MlstUseReq {
    /// builtin (pre-2025) or own
    #[serde(rename = "use")]
    which: String,
}

async fn mlst_use(State(a): State<App>, Json(b): Json<MlstUseReq>) -> Api {
    mlstdb::choose(&a.cat, &b.which)?;
    ok(mlstdb::status(&a.cat)?)
}

#[derive(Deserialize)]
struct MlstKeyReq {
    key: String,
}

async fn mlst_key(State(a): State<App>, Json(b): Json<MlstKeyReq>) -> Api {
    ok(tokio::task::spawn_blocking(move || mlstdb::add_key(&a.cat, &b.key)).await??)
}

#[derive(Deserialize)]
struct MlstOauthReq {
    client_key: String,
    client_secret: String,
}

async fn mlst_oauth_start(State(a): State<App>, Json(b): Json<MlstOauthReq>) -> Api {
    ok(tokio::task::spawn_blocking(move || mlstdb::oauth_start(&a.cat, &b.client_key, &b.client_secret)).await??)
}

#[derive(Deserialize)]
struct MlstCodeReq {
    code: String,
}

async fn mlst_oauth_finish(State(a): State<App>, Json(b): Json<MlstCodeReq>) -> Api {
    ok(tokio::task::spawn_blocking(move || mlstdb::oauth_finish(&a.cat, &b.code)).await??)
}

async fn mlst_update(State(a): State<App>) -> Api {
    ok(tokio::task::spawn_blocking(move || mlstdb::update(&a.cat)).await??)
}

async fn mlst_check(State(a): State<App>) -> Api {
    ok(tokio::task::spawn_blocking(move || mlstdb::check(&a.cat)).await??)
}

async fn mlst_remove(State(a): State<App>) -> Api {
    tokio::task::spawn_blocking(move || mlstdb::remove(&a.cat)).await??;
    ok(json!({"ok": true}))
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
            } else if name.ends_with(".gz") || name.ends_with(".fastq") || name.ends_with(".fq") || is_fasta(&name) {
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
    /// "assemblies": the folder's FASTA files, one sample each; else paired reads
    #[serde(default)]
    input_type: String,
}

/// The samples a run would have from this folder.
async fn pair_reads(Json(b): Json<ReadsReq>) -> Api {
    let found = find_input(&native_path(&b.input), &b.input_type, b.pairing.as_ref());
    let problems = input_problems(&found);
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
#[cfg(target_os = "linux")]
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
    let mut child = cmd.spawn()?;
    // collect it when it ends, so a run that stopped never looks as if it were still going
    std::thread::spawn(move || {
        let _ = child.wait();
    });
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

#[derive(Deserialize)]
struct SetUpReq {
    /// a folder of reads to try the command on (empty: don't try it)
    #[serde(default)]
    reads_folder: String,
}

/// "Set it up for me": the command from the tool's help text, a try-out on one
/// sample, the outputs from what it made.
async fn set_up_tool(State(a): State<App>, UrlPath(id): UrlPath<String>, Json(b): Json<SetUpReq>) -> Api {
    ok(tool_job(&a.cat, id, "setup", "Set the tool up", move |cat, id, r| {
        autosetup::set_up(cat, id, &b.reads_folder, r)
    }))
}

/// Build a container for a tool that has none, from its repository.
async fn build_tool(State(a): State<App>, UrlPath(id): UrlPath<String>) -> Api {
    ok(tool_job(
        &a.cat,
        id,
        "build",
        "Build the tool's container",
        build::build,
    ))
}

/// A job on one tool (help, try, set up, build). What it says is kept, so a
/// tool report can show it later, also after the app was closed.
fn tool_job(
    cat: &Catalog,
    id: String,
    kind: &'static str,
    title: &str,
    work: impl FnOnce(&Catalog, &str, &jobs::Reporter) -> anyhow::Result<()> + Send + 'static,
) -> jobs::Job {
    let cat = cat.clone();
    jobs::spawn(title, move |r| {
        let done = work(&cat, &id, r);
        toolreport::keep_log(&cat, &id, kind, &r.lines(), done.as_ref().err());
        done
    })
}

/// A tool report for when adding or setting up a tool does not work: one text
/// file to save and send (nothing private: names, keys and addresses are replaced).
async fn tool_report(State(a): State<App>, UrlPath(id): UrlPath<String>) -> Api {
    let text = tokio::task::spawn_blocking(move || {
        let computer = hardware::check(&a.cat);
        toolreport::report(&a.cat, &id, &computer)
    })
    .await??;
    ok(json!({"text": text}))
}

/// A problem report for a run that failed or stopped: one text file to save
/// and send (no sequence data; sample and user names are replaced).
async fn run_report(State(a): State<App>, UrlPath(id): UrlPath<String>) -> Api {
    let text = tokio::task::spawn_blocking(move || {
        let computer = hardware::check(&a.cat);
        gwm_engine::diagnose::problem_report(&a.cat, &id, &computer)
    })
    .await??;
    ok(json!({"text": text}))
}
