//! Run folders (port of v1 `app/runs.py`). Each run gets <home>/runs/<run_id>/:
//!   workflow.yml, plan.json, tools/   the workflow exactly as it ran
//!   samples.csv                       the read pairs (sample,r1,r2)
//!   run.json                          status, options, per-step progress
//!   results/                          samples/, combined/, summary/, pipeline_info/
//! The work folder is <home>/work/<run_id> (on the Linux disk: much faster
//! than a Windows drive under WSL); deleted after success when cleanup is on.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use chrono::Local;
use gwm_core::reads::{find_samples, pairing_problems, Pairing};
use gwm_core::{catalog, compiler, paths, Catalog};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::backend::Backend;
use crate::executor::{Executor, Outcome, RunSpec, Status, TaskEvent};
use crate::provenance::{self, RunInfo};
use crate::resources::Limits;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StartOptions {
    pub name: Option<String>,
    pub version: Option<u32>,
    #[serde(default)]
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

pub fn runs_dir(cat: &Catalog) -> PathBuf {
    cat.home.join("runs")
}

pub fn run_path(cat: &Catalog, run_id: &str) -> Result<PathBuf> {
    if run_id.is_empty()
        || !run_id.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        || run_id.starts_with('.')
    {
        bail!("bad run id");
    }
    Ok(runs_dir(cat).join(run_id))
}

pub fn load(cat: &Catalog, run_id: &str) -> Result<Value> {
    let f = run_path(cat, run_id)?.join("run.json");
    Ok(serde_json::from_str(
        &fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?,
    )?)
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
    let label: String = opts
        .name
        .clone()
        .unwrap_or_else(|| wf.id.clone())
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let run_id = format!(
        "{}_{}",
        started.format("%Y%m%d-%H%M%S"),
        label.chars().take(40).collect::<String>()
    );
    let rdir = run_path(cat, &run_id)?;
    fs::create_dir_all(&rdir)?;

    // the workflow exactly as it ran, and the tools it used
    catalog::dump_yaml(&wf, &rdir.join("workflow.yml"))?;
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
        let dest = rdir.join("tools").join(&r.tool.bundle);
        if !dest.exists() {
            copy_dir(&r.tool.dir, &dest)?;
        }
    }
    fs::write(rdir.join("plan.json"), serde_json::to_string_pretty(&plan)?)?;
    let mut csv = String::from("sample,r1,r2\n");
    for s in &found.samples {
        csv += &format!("{},{},{}\n", s.sample, s.r1.display(), s.r2.as_ref().unwrap().display());
    }
    fs::write(rdir.join("samples.csv"), csv)?;

    let work = cat.home.join("work").join(&run_id);
    let results = rdir.join("results");
    let limits = Limits::detect(opts.max_cpus, opts.max_memory_gb);
    let command = format!("gwm run {} {}", wf.id, input_dir.display());
    let run = json!({
        "id": run_id, "name": opts.name.clone().unwrap_or_else(|| wf.name.clone()),
        "workflow": {"id": wf.id, "version": wf.saved_version, "name": wf.name},
        "input": input_dir, "samples": found.samples.iter().map(|s| &s.sample).collect::<Vec<_>>(),
        "n_samples": found.samples.len(),
        "pairing": {"mode": found.mode, "r1": found.r1, "r2": found.r2, "unpaired": found.unpaired},
        "cleanup": opts.cleanup, "steps_off": steps_off, "status": "running",
        "started": started.format("%Y-%m-%dT%H:%M:%S").to_string(), "finished": null,
        "results": results, "work": work, "steps": {}, "error": null,
        "engine": {"version": env!("CARGO_PKG_VERSION"), "containers": backend.name()},
        "limits": limits, "pid": std::process::id(), "command": command,
    });
    save(&rdir, &run)?;

    let spec = RunSpec {
        run_id: run_id.clone(),
        plan,
        samples: found.samples.clone(),
        run_dir: rdir.clone(),
        work_dir: work.clone(),
        results_dir: results.clone(),
        db_dir: paths::db_dir(&cat.home),
        limits,
    };
    execute(cat, run, spec, opts.cleanup, &command, backend, stop, on_event)
}

/// Run a stopped or failed run again in its own folder: finished tasks are
/// taken from the work folder (cached), only the rest runs.
pub fn resume(
    cat: &Catalog,
    run_id: &str,
    backend: &dyn Backend,
    stop: &AtomicBool,
    on_event: &(dyn Fn(&TaskEvent) + Sync),
) -> Result<Value> {
    let rdir = run_path(cat, run_id)?;
    let mut run = load(cat, run_id)?;
    if run["status"] == "running" {
        bail!("run {run_id} is still running");
    }
    let plan = serde_json::from_str(&fs::read_to_string(rdir.join("plan.json")).context("reading plan.json")?)?;
    let mut samples = Vec::new();
    for line in fs::read_to_string(rdir.join("samples.csv"))?.lines().skip(1) {
        let mut parts = line.splitn(3, ',');
        let (Some(s), Some(r1), Some(r2)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        samples.push(gwm_core::reads::Sample {
            sample: s.into(),
            r1: r1.into(),
            r2: Some(r2.into()),
        });
    }
    let path = |k: &str| PathBuf::from(run[k].as_str().unwrap_or_default());
    let spec = RunSpec {
        run_id: run_id.to_string(),
        plan,
        samples,
        run_dir: rdir.clone(),
        work_dir: path("work"),
        results_dir: path("results"),
        db_dir: paths::db_dir(&cat.home),
        limits: serde_json::from_value(run["limits"].clone()).unwrap_or_else(|_| Limits::detect(0, 0)),
    };
    for k in ["finished", "error", "exit_code"] {
        run[k] = Value::Null;
    }
    run["status"] = "running".into();
    run["resumed"] = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string().into();
    run["pid"] = std::process::id().into();
    save(&rdir, &run)?;
    let cleanup = run["cleanup"].as_bool().unwrap_or(false);
    let command = format!("gwm resume {run_id}");
    execute(cat, run, spec, cleanup, &command, backend, stop, on_event)
}

#[allow(clippy::too_many_arguments)]
fn execute(
    cat: &Catalog,
    run: Value,
    spec: RunSpec,
    cleanup: bool,
    command: &str,
    backend: &dyn Backend,
    stop: &AtomicBool,
    on_event: &(dyn Fn(&TaskEvent) + Sync),
) -> Result<Value> {
    let started = Local::now();
    let rdir = spec.run_dir.clone();
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
        let _ = save(&rdir, &run);
    };

    let outcome: Result<Outcome> = Executor::new(&spec, backend, &record).and_then(|ex| ex.run(stop));
    let completed = Local::now();
    let mut run = state.into_inner().unwrap();
    let outcome = match outcome {
        Ok(o) => o,
        Err(e) => Outcome {
            success: false,
            stopped: false,
            error: Some(format!("{e:#}")),
            trace: vec![],
            versions: vec![],
            ran: vec![],
        },
    };
    let info = RunInfo {
        name: run["name"].as_str().unwrap_or_default().to_string(),
        started,
        completed,
        command_line: command.to_string(),
        root: cat.root.clone(),
        params: json!({
            "input": input_dir, "samplesheet": rdir.join("samples.csv"), "outdir": results,
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
    save(&rdir, &run)?;
    if outcome.success && cleanup {
        fs::remove_dir_all(&work).ok();
    }
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
    let mut dirs: Vec<PathBuf> = fs::read_dir(runs_dir(cat))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.join("run.json").is_file())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs.reverse();
    dirs.iter()
        .filter_map(|d| serde_json::from_str::<Value>(&fs::read_to_string(d.join("run.json")).ok()?).ok())
        .map(|r| {
            let mut m = Map::new();
            for k in ["id", "name", "workflow", "status", "started", "finished", "n_samples"] {
                m.insert(k.into(), r.get(k).cloned().unwrap_or(Value::Null));
            }
            Value::Object(m)
        })
        .collect()
}

/// Delete a run: its folder and work folder. The reads are never touched.
pub fn delete(cat: &Catalog, run_id: &str) -> Result<()> {
    let dir = run_path(cat, run_id)?;
    if !dir.join("run.json").is_file() {
        return Err(anyhow!("unknown run {run_id}"));
    }
    fs::remove_dir_all(cat.home.join("work").join(run_id)).ok();
    fs::remove_dir_all(dir)?;
    Ok(())
}
