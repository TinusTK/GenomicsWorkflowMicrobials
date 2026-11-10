//! Run folders. Every run is ONE folder, by default under
//! Documents › Genomics Workflow Microbials, named "<date time> <run name>":
//!
//!   run.json            status, options, per-step progress (the app reads this)
//!   Results/            samples/, combined/, summary/, pipeline_info/
//!   Run details/        workflow.yml, plan.json, samples.csv, tools/, exec.log:
//!                       exactly what ran, for the lab record
//!   README.txt          what is where
//!
//! Deleting the folder (in the app or by hand) deletes the run. The big working
//! files of the steps live on the fast disk (`<home>/work/<run>`; under WSL the
//! Linux disk is far faster than Documents): they are deleted after a
//! successful run, and whenever their run folder is gone.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use chrono::Local;
use gwm_core::reads::{find_samples, pairing_problems, Pairing, Sample};
use gwm_core::{catalog, compiler, paths, Catalog};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::backend::Backend;
use crate::executor::{Executor, Outcome, RunSpec, Status, TaskEvent};
use crate::provenance::{self, RunInfo};
use crate::resources::Limits;

pub const RESULTS: &str = "Results";
pub const DETAILS: &str = "Run details";
/// In a work folder: the run folder it belongs to (to clean up orphans).
const OWNER: &str = ".run_folder";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartOptions {
    pub name: Option<String>,
    pub version: Option<u32>,
    /// delete the working files after a successful run (results are kept)
    #[serde(default = "yes")]
    pub cleanup: bool,
    #[serde(default)]
    pub steps_off: Vec<String>,
    pub pairing: Option<Pairing>,
    /// 0 = detect
    #[serde(default)]
    pub max_cpus: u32,
    #[serde(default)]
    pub max_memory_gb: u32,
}

fn yes() -> bool {
    true
}

impl Default for StartOptions {
    fn default() -> Self {
        StartOptions {
            name: None,
            version: None,
            cleanup: true,
            steps_off: vec![],
            pairing: None,
            max_cpus: 0,
            max_memory_gb: 0,
        }
    }
}

/// A run's id is its folder name; never a path.
pub fn run_path(cat: &Catalog, run_id: &str) -> Result<PathBuf> {
    let bad = run_id.is_empty()
        || run_id.starts_with('.')
        || run_id
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control());
    if bad {
        bail!("bad run id");
    }
    Ok(cat.runs.join(run_id))
}

pub fn load(cat: &Catalog, run_id: &str) -> Result<Value> {
    let f = run_path(cat, run_id)?.join("run.json");
    let text = fs::read_to_string(&f).map_err(|_| anyhow!("unknown run {run_id}"))?;
    Ok(serde_json::from_str(&text)?)
}

fn save(dir: &Path, run: &Value) -> Result<()> {
    let tmp = dir.join("run.json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(run)?)?;
    fs::rename(&tmp, dir.join("run.json"))?;
    Ok(())
}

fn rank(s: &str) -> u8 {
    match s {
        "SUBMITTED" => 1,
        "RUNNING" => 2,
        _ => 3,
    }
}

/// "2026-11-10 09-54 Week 45 controls": sorts by date, safe on every system.
fn folder_name(cat: &Catalog, when: &chrono::DateTime<Local>, name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim().trim_matches('.').chars().take(60).collect::<String>();
    let base = format!("{} {}", when.format("%Y-%m-%d %H-%M"), clean)
        .trim()
        .to_string();
    let mut id = base.clone();
    let mut n = 2;
    while cat.runs.join(&id).exists() || cat.home.join("work").join(work_name(&id)).exists() {
        id = format!("{base} ({n})");
        n += 1;
    }
    id
}

/// Make the run folder and run it to the end, updating run.json as tasks move on.
/// `on_event` also gets every task event (console progress, the UI).
pub fn start(
    cat: &Catalog,
    workflow: &str,
    input_dir: &Path,
    opts: &StartOptions,
    backend: &dyn Backend,
    stop: &AtomicBool,
    on_event: &(dyn Fn(&TaskEvent) + Sync),
) -> Result<Value> {
    let run = prepare(cat, workflow, input_dir, opts, backend.name())?;
    resume(cat, run["id"].as_str().unwrap_or_default(), backend, stop, on_event)
}

/// Make the run folder (status "queued"); [`resume`] runs it. The app starts
/// that in its own process, so closing the window never stops a run.
pub fn prepare(
    cat: &Catalog,
    workflow: &str,
    input_dir: &Path,
    opts: &StartOptions,
    containers: &str,
) -> Result<Value> {
    let started = Local::now();
    let mut wf = cat.load_workflow(workflow, opts.version)?;
    let input_dir = fs::canonicalize(input_dir).with_context(|| format!("reads folder {}", input_dir.display()))?;
    let found = find_samples(&input_dir, opts.pairing.as_ref());
    if found.samples.is_empty() {
        bail!("no paired reads found in {}", input_dir.display());
    }
    let problems = pairing_problems(&found);
    if !problems.is_empty() {
        bail!("{}", problems.join(" "));
    }
    let tools = cat.load_tools()?;
    compiler::resolve(&wf, &tools)?;
    let name = opts
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| wf.name.clone());
    let run_id = folder_name(cat, &started, &name);
    let rdir = run_path(cat, &run_id)?;
    let details = rdir.join(DETAILS);
    fs::create_dir_all(&details).with_context(|| format!("making the run folder {}", rdir.display()))?;

    // the workflow exactly as it ran, and the tools it used
    catalog::dump_yaml(&wf, &details.join("workflow.yml"))?;
    let steps_off: Vec<String> = opts
        .steps_off
        .iter()
        .filter(|s| wf.steps.contains_key(*s))
        .cloned()
        .collect();
    for s in &steps_off {
        wf.steps[s].enabled = Some(false);
    }
    let plan = compiler::plan(&wf, &tools, &cat.load_databases()?)?;
    for r in compiler::resolve(&wf, &tools)? {
        let dest = details.join("tools").join(&r.tool.bundle);
        if !dest.exists() {
            copy_dir(&r.tool.dir, &dest)?;
        }
    }
    fs::write(details.join("plan.json"), serde_json::to_string_pretty(&plan)?)?;
    let mut csv = String::from("sample,r1,r2\n");
    for s in &found.samples {
        csv += &format!("{},{},{}\n", s.sample, s.r1.display(), s.r2.as_ref().unwrap().display());
    }
    fs::write(details.join("samples.csv"), csv)?;
    fs::write(rdir.join("README.txt"), readme(&name, &wf.name, found.samples.len()))?;

    let work = cat.home.join("work").join(work_name(&run_id));
    let results = rdir.join(RESULTS);
    let limits = Limits::detect(opts.max_cpus, opts.max_memory_gb);
    let command = format!("gwm run {} {}", wf.id, input_dir.display());
    let run = json!({
        "id": run_id, "name": name,
        "workflow": {"id": wf.id, "version": wf.saved_version, "name": wf.name},
        "input": input_dir, "samples": found.samples.iter().map(|s| &s.sample).collect::<Vec<_>>(),
        "n_samples": found.samples.len(),
        "pairing": {"mode": found.mode, "r1": found.r1, "r2": found.r2, "unpaired": found.unpaired},
        "cleanup": opts.cleanup, "steps_off": steps_off, "status": "queued",
        "started": started.format("%Y-%m-%dT%H:%M:%S").to_string(), "finished": null,
        "folder": rdir, "results": results, "work": work, "steps": {}, "error": null,
        "engine": {"version": env!("CARGO_PKG_VERSION"), "containers": containers},
        "limits": limits, "pid": null, "command": command,
    });
    save(&rdir, &run)?;
    Ok(run)
}

fn readme(name: &str, workflow: &str, n: usize) -> String {
    format!(
        "Genomics Workflow Microbials run: {name}\n\
         Workflow: {workflow}; {n} sample(s).\n\n\
         Results/       the results: samples/<sample>/<step>, combined/, summary/ (the tables)\n\
         Results/pipeline_info/provenance.json   every tool version, container and setting used\n\
         Run details/   the workflow, read pairs and tools exactly as they ran, and the log\n\
         run.json       the app's record of this run\n\n\
         To delete this run, delete this folder (or use Delete in the app).\n\
         Your read files are not in here and are never touched.\n"
    )
}

/// Is the process that runs this run still there?
pub fn pid_alive(pid: Option<u64>) -> bool {
    let Some(pid) = pid else { return false };
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks the process exists
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// run.json, with a run whose process is gone (computer restarted, killed)
/// marked "interrupted" so it can be resumed.
pub fn refresh(cat: &Catalog, run_id: &str) -> Result<Value> {
    let mut run = load(cat, run_id)?;
    if run["status"] == "running" && !pid_alive(run["pid"].as_u64()) {
        run["status"] = "interrupted".into();
        run["error"] =
            "The run stopped unexpectedly (the computer restarted or the process was killed). Resume it to carry on."
                .into();
        save(&run_path(cat, run_id)?, &run)?;
    }
    Ok(run)
}

/// Ask a running run to stop: its process sees the `.stop` file and kills the running steps.
pub fn request_stop(cat: &Catalog, run_id: &str) -> Result<bool> {
    let run = refresh(cat, run_id)?;
    if run["status"] != "running" && run["status"] != "queued" {
        return Ok(false);
    }
    fs::write(run_path(cat, run_id)?.join(".stop"), "")?;
    Ok(true)
}

fn read_samples(details: &Path) -> Result<Vec<Sample>> {
    let mut samples = Vec::new();
    for line in fs::read_to_string(details.join("samples.csv"))?.lines().skip(1) {
        let mut parts = line.splitn(3, ',');
        if let (Some(s), Some(r1), Some(r2)) = (parts.next(), parts.next(), parts.next()) {
            samples.push(Sample {
                sample: s.into(),
                r1: r1.into(),
                r2: Some(r2.into()),
            });
        }
    }
    Ok(samples)
}

/// Run a prepared, stopped or failed run in its own folder: finished steps are
/// taken from the work folder (cached), only the rest runs.
pub fn resume(
    cat: &Catalog,
    run_id: &str,
    backend: &dyn Backend,
    stop: &AtomicBool,
    on_event: &(dyn Fn(&TaskEvent) + Sync),
) -> Result<Value> {
    let rdir = run_path(cat, run_id)?;
    let details = rdir.join(DETAILS);
    let mut run = refresh(cat, run_id)?;
    if run["status"] == "running" {
        bail!("run {run_id} is still running");
    }
    fs::remove_file(rdir.join(".stop")).ok();
    let plan = serde_json::from_str(&fs::read_to_string(details.join("plan.json")).context("reading plan.json")?)?;
    let path = |k: &str| PathBuf::from(run[k].as_str().unwrap_or_default());
    let spec = RunSpec {
        run_id: run_id.to_string(),
        plan,
        samples: read_samples(&details)?,
        run_dir: details.clone(),
        work_dir: path("work"),
        results_dir: path("results"),
        db_dir: paths::db_dir(&cat.home),
        limits: serde_json::from_value(run["limits"].clone()).unwrap_or_else(|_| Limits::detect(0, 0)),
    };
    fs::create_dir_all(&spec.work_dir)?;
    fs::write(spec.work_dir.join(OWNER), rdir.to_string_lossy().as_bytes())?;
    for k in ["finished", "error", "exit_code"] {
        run[k] = Value::Null;
    }
    if run["status"] != "queued" {
        run["resumed"] = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string().into();
    }
    run["status"] = "running".into();
    run["pid"] = std::process::id().into();
    save(&rdir, &run)?;
    let cleanup = run["cleanup"].as_bool().unwrap_or(true);
    let command = format!("gwm resume \"{run_id}\"");
    execute(cat, &rdir, run, spec, cleanup, &command, backend, stop, on_event)
}

#[allow(clippy::too_many_arguments)]
fn execute(
    cat: &Catalog,
    rdir: &Path,
    run: Value,
    spec: RunSpec,
    cleanup: bool,
    command: &str,
    backend: &dyn Backend,
    stop: &AtomicBool,
    on_event: &(dyn Fn(&TaskEvent) + Sync),
) -> Result<Value> {
    let started = Local::now();
    let (work, results) = (spec.work_dir.clone(), spec.results_dir.clone());
    let input_dir = run["input"].clone();
    let state = Mutex::new(run);
    let record = |e: &TaskEvent| {
        on_event(e);
        let mut run = state.lock().unwrap();
        let status = serde_json::to_value(e.status).unwrap();
        let st = run["steps"]
            .as_object_mut()
            .unwrap()
            .entry(e.step.clone())
            .or_insert_with(|| json!({"tasks": {}}));
        let cur = st["tasks"]
            .as_object_mut()
            .unwrap()
            .entry(e.tag.clone())
            .or_insert_with(|| json!({}));
        if rank(status.as_str().unwrap()) < rank(cur["status"].as_str().unwrap_or("")) {
            return;
        }
        let now = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let cur = cur.as_object_mut().unwrap();
        if e.status == Status::Running && !cur.contains_key("started_at") {
            cur.insert("started_at".into(), now.clone().into());
        }
        if !matches!(e.status, Status::Submitted | Status::Running) {
            cur.insert("finished_at".into(), now.into());
        }
        cur.insert("status".into(), status);
        cur.insert("hash".into(), e.hash.clone().into());
        cur.insert("cpus".into(), e.cpus.into());
        cur.insert("memory_gb".into(), e.memory_gb.into());
        if let Some(x) = e.exit {
            cur.insert("exit".into(), x.into());
        }
        let _ = save(rdir, &run);
    };

    // stop when asked from outside (Ctrl+C) or by the app (a .stop file in the run folder)
    let halt = AtomicBool::new(false);
    let finished = AtomicBool::new(false);
    let outcome: Result<Outcome> = std::thread::scope(|s| {
        s.spawn(|| {
            while !finished.load(Ordering::Relaxed) {
                if stop.load(Ordering::Relaxed) || rdir.join(".stop").exists() {
                    halt.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        });
        let r = Executor::new(&spec, backend, &record).and_then(|ex| ex.run(&halt));
        finished.store(true, Ordering::Relaxed);
        r
    });
    fs::remove_file(rdir.join(".stop")).ok();
    let completed = Local::now();
    let mut run = state.into_inner().unwrap();
    let outcome = outcome.unwrap_or_else(|e| Outcome {
        success: false,
        stopped: false,
        error: Some(format!("{e:#}")),
        trace: vec![],
        versions: vec![],
        ran: vec![],
    });
    let info = RunInfo {
        name: run["name"].as_str().unwrap_or_default().to_string(),
        started,
        completed,
        command_line: command.to_string(),
        root: cat.root.clone(),
        params: json!({
            "input": input_dir, "samplesheet": spec.run_dir.join("samples.csv"), "outdir": results,
            "cleanup": cleanup, "steps_off": run["steps_off"], "reads_pattern": spec.plan.reads_pattern,
        }),
    };
    if let Err(e) = provenance::write(&spec, &outcome, backend, &info) {
        eprintln!("provenance: {e:#}");
    }
    run["status"] = (if outcome.success {
        "succeeded"
    } else if outcome.stopped {
        "stopped"
    } else {
        "failed"
    })
    .into();
    run["exit_code"] = (if outcome.success { 0 } else { 1 }).into();
    run["finished"] = completed.format("%Y-%m-%dT%H:%M:%S").to_string().into();
    run["error"] = outcome.error.clone().map_or(Value::Null, Value::from);
    run["ran"] = json!(outcome.ran);
    if outcome.success && cleanup {
        fs::remove_dir_all(&work).ok();
        run["work_deleted"] = true.into();
    }
    save(rdir, &run)?;
    Ok(run)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    for e in walkdir::WalkDir::new(from).follow_links(true) {
        let e = e?;
        let dest = to.join(e.path().strip_prefix(from)?);
        if e.file_type().is_dir() {
            fs::create_dir_all(&dest)?;
        } else {
            fs::copy(e.path(), &dest)?;
        }
    }
    Ok(())
}

/// Runs, newest first: the fields the Runs page lists.
pub fn list(cat: &Catalog) -> Vec<Value> {
    let mut runs: Vec<Value> = fs::read_dir(&cat.runs)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    serde_json::from_str::<Value>(&fs::read_to_string(e.path().join("run.json")).ok()?).ok()
                })
                .collect()
        })
        .unwrap_or_default();
    runs.sort_by(|a, b| b["started"].as_str().cmp(&a["started"].as_str()));
    runs.into_iter()
        .map(|r| {
            let mut m = Map::new();
            for k in [
                "id",
                "name",
                "workflow",
                "status",
                "started",
                "finished",
                "n_samples",
                "folder",
            ] {
                m.insert(k.into(), r.get(k).cloned().unwrap_or(Value::Null));
            }
            Value::Object(m)
        })
        .collect()
}

/// Delete a run: its folder and its working files. The reads are never touched.
pub fn delete(cat: &Catalog, run_id: &str) -> Result<()> {
    let dir = run_path(cat, run_id)?;
    let run = load(cat, run_id)?;
    if let Some(work) = run["work"].as_str() {
        fs::remove_dir_all(work).ok();
    }
    fs::remove_dir_all(dir)?;
    Ok(())
}

/// Delete working files whose run folder is gone (deleted by hand). Returns how many.
pub fn clean_orphan_work(cat: &Catalog) -> usize {
    let mut n = 0;
    for e in fs::read_dir(cat.home.join("work")).into_iter().flatten().flatten() {
        let Ok(owner) = fs::read_to_string(e.path().join(OWNER)) else {
            continue;
        };
        let owner = PathBuf::from(owner.trim());
        if !owner.join("run.json").exists() && fs::remove_dir_all(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

/// Everything the Run page shows: run.json, the plan's steps (to draw the
/// pipeline) and the result files.
pub fn detail(cat: &Catalog, run_id: &str) -> Result<Value> {
    let mut run = refresh(cat, run_id)?;
    let rdir = run_path(cat, run_id)?;
    if let Ok(text) = fs::read_to_string(rdir.join(DETAILS).join("plan.json")) {
        let plan: Value = serde_json::from_str(&text)?;
        let steps: Vec<Value> = plan["steps"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| {
                        let sources: Vec<Value> = s["inputs"]
                            .as_object()
                            .map(|m| {
                                m.values()
                                    .flat_map(|i| i["sources"].as_array().cloned().unwrap_or_default())
                                    .collect()
                            })
                            .unwrap_or_default();
                        json!({
                            "id": s["id"], "label": s["label"], "tool": s["tool_name"], "version": s["tool_version"],
                            "per_sample": s["per_sample"], "enabled": s["enabled"], "runnable": s["runnable"],
                            "inputs": sources,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        run["plan_steps"] = json!(steps);
    }
    let results = PathBuf::from(run["results"].as_str().unwrap_or_default());
    let mut files = Vec::new();
    for e in walkdir::WalkDir::new(&results)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .flatten()
    {
        if e.file_type().is_file() {
            let rel = e.path().strip_prefix(&results)?.to_string_lossy().replace('\\', "/");
            files.push(json!({"path": rel, "size": e.metadata().map(|m| m.len()).unwrap_or(0)}));
        }
    }
    run["files"] = json!(files);
    Ok(run)
}

/// A file under a run's results folder (never outside it).
pub fn result_file(cat: &Catalog, run_id: &str, rel: &str) -> Result<PathBuf> {
    let run = load(cat, run_id)?;
    let results = PathBuf::from(run["results"].as_str().unwrap_or_default());
    let real = fs::canonicalize(results.join(rel)).map_err(|_| anyhow!("no file {rel}"))?;
    if !real.starts_with(fs::canonicalize(&results)?) || !real.is_file() {
        bail!("no file {rel}");
    }
    Ok(real)
}

/// The working folder's name: the run's name with only letters, digits, `.`,
/// `_` and `-` (some tools, e.g. BLAST inside SISTR, break on spaces in paths).
pub fn work_name(run_id: &str) -> String {
    run_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}
