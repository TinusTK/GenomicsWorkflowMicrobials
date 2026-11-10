//! pipeline_info/ at the end of a run: provenance.json (what made every result:
//! app version, tool versions, container images and digests, settings),
//! tool_versions.yml and trace.tsv. Port of v1 `lib/Provenance.groovy`.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::Result;
use chrono::{DateTime, Local};
use indexmap::IndexMap;
use serde_json::{json, Value};

use crate::backend::Backend;
use crate::executor::{Outcome, RunSpec};

pub const PIPELINE_NAME: &str = "Genomics Workflow Microbials";
pub const REPRODUCES: &str = "Galaxy Australia: Genomic Assembly and analysis - RDH version 6";

/// About the run, from whoever started it.
#[derive(Debug, Clone)]
pub struct RunInfo {
    pub name: String,
    pub started: DateTime<Local>,
    pub completed: DateTime<Local>,
    pub command_line: String,
    /// folder the app's tools/ and workflows/ came from (for the git commit, if any)
    pub root: std::path::PathBuf,
    /// settings the run was started with (input, steps off, cleanup, ...)
    pub params: Value,
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn ts(t: &Option<DateTime<Local>>) -> String {
    t.map(|t| t.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_else(|| "-".into())
}

pub fn write(spec: &RunSpec, outcome: &Outcome, backend: &dyn Backend, info: &RunInfo) -> Result<()> {
    let dir = spec.results_dir.join("pipeline_info");
    fs::create_dir_all(&dir)?;
    let stamp = info.started.format("%Y%m%d-%H%M%S");

    fs::write(
        dir.join("tool_versions.yml"),
        outcome.versions.iter().map(|l| format!("{l}\n")).collect::<String>(),
    )?;

    let mut tsv =
        String::from("task_id\thash\tname\tstatus\texit\tsubmit\tstart\tcomplete\tduration\tcpus\tmemory\tworkdir\n");
    for r in &outcome.trace {
        let dur = match (r.start, r.complete) {
            (Some(a), Some(b)) => format!("{:.1}s", (b - a).num_milliseconds() as f64 / 1000.0),
            _ => "-".into(),
        };
        tsv += &format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{dur}\t{}\t{} GB\t{}\n",
            r.task_id,
            r.hash,
            r.name,
            serde_json::to_value(r.status)?.as_str().unwrap_or(""),
            r.exit.map_or("-".into(), |e| e.to_string()),
            ts(&r.submit),
            ts(&r.start),
            ts(&r.complete),
            r.cpus,
            r.memory_gb,
            r.workdir
        );
    }
    fs::write(dir.join(format!("trace_{stamp}.tsv")), tsv)?;

    let mut tools = IndexMap::new();
    let mut notes = IndexMap::new();
    let mut containers = IndexMap::new();
    for line in &outcome.versions {
        let Some((k, v)) = line.split_once(':') else { continue };
        let (k, v) = (k.trim(), v.trim());
        if let Some(p) = k.strip_prefix("container.") {
            containers.insert(p.to_string(), json!({"image": v, "digest": backend.image_digest(v)}));
        } else if let Some(n) = k.strip_prefix("note.") {
            notes.insert(n.to_string(), v.to_string());
        } else {
            tools.insert(k.to_string(), v.to_string());
        }
    }
    tools.sort_keys();
    containers.sort_keys();
    let steps: IndexMap<&str, bool> = spec.plan.steps.iter().map(|s| (s.id.as_str(), s.enabled)).collect();
    let settings: IndexMap<&str, &IndexMap<String, String>> = spec
        .plan
        .steps
        .iter()
        .filter(|s| !s.settings.is_empty())
        .map(|s| (s.id.as_str(), &s.settings))
        .collect();
    let databases: IndexMap<&String, String> = spec
        .plan
        .databases
        .iter()
        .map(|(k, d)| (k, spec.db_dir.join(d).to_string_lossy().into_owned()))
        .collect();
    let duration = (info.completed - info.started).num_milliseconds() as f64 / 1000.0;
    let record = json!({
        "pipeline": {
            "name": PIPELINE_NAME,
            "version": env!("CARGO_PKG_VERSION"),
            "git_commit": git(&info.root, &["rev-parse", "HEAD"]),
            "git_dirty": git(&info.root, &["status", "--porcelain"]).map(|s| !s.is_empty()),
            "reproduces": REPRODUCES,
        },
        "run": {
            "name": info.name,
            "id": spec.run_id,
            "started": info.started.to_rfc3339(),
            "completed": info.completed.to_rfc3339(),
            "duration": format!("{duration:.1}s"),
            "success": outcome.success,
            "stopped": outcome.stopped,
            "command_line": info.command_line,
        },
        "engine": {"name": "gwm-engine", "version": env!("CARGO_PKG_VERSION"), "containers": backend.name()},
        "workflow": {"id": spec.plan.workflow_id, "version": spec.plan.workflow_version, "name": spec.plan.workflow_name},
        "tools": tools,
        // decisions made while running, e.g. "checkm.tree: reduced (auto; 17 GB RAM ...)"
        "notes": notes,
        "steps": steps,
        "settings": settings,
        "databases": databases,
        "containers": containers,
        "samples": spec.samples,
        "limits": spec.limits,
        "params": info.params,
    });
    fs::write(
        dir.join("provenance.json"),
        serde_json::to_string_pretty(&record)? + "\n",
    )?;
    Ok(())
}
