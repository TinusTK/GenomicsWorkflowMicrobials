//! Runs a plan: one task per (step, sample), or one per all-sample step, as soon
//! as its inputs are ready and there is room in the CPU/memory budget. Each task
//! gets a work folder named after a hash of everything that goes into it, so a
//! stopped or failed run picks up where it was (finished tasks are "cached").
//!
//! Same rules as the v1 Nextflow code:
//! - an input takes the first source whose step runs (`input` = the reads)
//! - a per-sample task needs each wired input for its own sample; when an
//!   optional output is missing for that sample, the task is skipped
//! - an all-sample task gets every sample's files in `in_<input>/`
//! - outputs under `out/` are copied to the results folder
//! - one failed task stops the run (running tasks are killed)

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Local};
use globset::GlobBuilder;
use gwm_core::compiler::{PlanInput, PlanStep, INPUT};
use gwm_core::reads::Sample;
use gwm_core::Plan;
use indexmap::IndexMap;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::backend::{Backend, TaskRun};
use crate::resources::{task_memory_gb, Limits};

/// Everything a run needs. `plan` already has the steps switched off for this
/// run (`enabled: false`, dependents not runnable).
#[derive(Debug, Clone)]
pub struct RunSpec {
    pub run_id: String,
    pub plan: Plan,
    /// the samples: read pairs (every r2 present), or FASTA files (no r2)
    pub samples: Vec<Sample>,
    /// holds `tools/<bundle>/`: the tools exactly as they ran
    pub run_dir: PathBuf,
    pub work_dir: PathBuf,
    pub results_dir: PathBuf,
    pub db_dir: PathBuf,
    pub limits: Limits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    Submitted,
    Running,
    Completed,
    Cached,
    Skipped,
    Failed,
    Aborted,
}

/// Progress for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct TaskEvent {
    pub step: String,
    /// sample id, or "all"
    pub tag: String,
    pub status: Status,
    pub hash: String,
    pub cpus: u32,
    pub memory_gb: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One line of pipeline_info/trace.tsv.
#[derive(Debug, Clone, Serialize)]
pub struct TraceRow {
    pub task_id: usize,
    pub hash: String,
    pub name: String,
    pub status: Status,
    pub exit: Option<i32>,
    pub submit: Option<DateTime<Local>>,
    pub start: Option<DateTime<Local>>,
    pub complete: Option<DateTime<Local>>,
    pub cpus: u32,
    pub memory_gb: u32,
    pub workdir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub success: bool,
    pub stopped: bool,
    pub error: Option<String>,
    pub trace: Vec<TraceRow>,
    /// "key: value" lines from every task's versions.yml, sorted, no duplicates
    pub versions: Vec<String>,
    /// steps that ran (or were cached) in this run
    pub ran: Vec<String>,
}

#[derive(Debug, Clone)]
enum State {
    Pending,
    Running,
    /// output name -> files (sorted)
    Done(IndexMap<String, Vec<PathBuf>>),
    Skipped,
    Failed,
}

struct Task {
    step: usize,
    /// None = all samples
    sample: Option<usize>,
    deps: Vec<usize>,
    state: State,
    row: TraceRow,
}

impl Task {
    fn tag(&self, spec: &RunSpec) -> String {
        self.sample.map_or("all".into(), |s| spec.samples[s].sample.clone())
    }
}

/// The source an input reads from: "input", or (step index, output name).
#[derive(Debug, Clone)]
enum Source {
    Reads,
    Step(usize, String),
}

fn pick(plan: &Plan, sources: &[String]) -> Option<Source> {
    for src in sources {
        if src == INPUT {
            return Some(Source::Reads);
        }
        let (up, out) = src.split_once('.').unwrap_or((src, ""));
        if let Some(i) = plan.steps.iter().position(|s| s.id == up) {
            if plan.steps[i].runnable {
                return Some(Source::Step(i, out.to_string()));
            }
        }
    }
    None
}

/// The sources an input reads: the first that runs, or (a `collect` input) every one that runs.
fn picks(plan: &Plan, input: &PlanInput) -> Vec<Source> {
    if !input.collect {
        return pick(plan, &input.sources).into_iter().collect();
    }
    input
        .sources
        .iter()
        .filter_map(|s| pick(plan, std::slice::from_ref(s)))
        .collect()
}

fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn now() -> DateTime<Local> {
    Local::now()
}

/// What a tool's user data says is in use: the small files at its top (a
/// choice, a version). A change there runs the step again on a resume.
fn user_data_id(dir: &Path) -> String {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && fs::metadata(p).is_ok_and(|m| m.len() < 4096))
        .collect();
    files.sort();
    files
        .iter()
        .map(|p| format!("{}={}", p.display(), fs::read_to_string(p).unwrap_or_default().trim()))
        .collect::<Vec<_>>()
        .join("|")
}

fn file_id(p: &Path) -> String {
    let m = fs::metadata(p);
    let size = m.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = m
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{}|{size}|{mtime}", p.display())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        for e in walkdir::WalkDir::new(from).follow_links(true) {
            let e = e?;
            let dest = to.join(e.path().strip_prefix(from)?);
            if e.file_type().is_dir() {
                fs::create_dir_all(&dest)?;
            } else {
                if let Some(d) = dest.parent() {
                    fs::create_dir_all(d)?;
                }
                fs::copy(e.path(), &dest)?;
            }
        }
    } else {
        if let Some(d) = to.parent() {
            fs::create_dir_all(d)?;
        }
        fs::copy(from, to).with_context(|| format!("copying {}", from.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}

#[cfg(not(unix))]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    fs::copy(target, at).map(|_| ())
}

/// A task with its folder made and its script written, ready for the backend.
struct Prepared {
    run: TaskRun,
    hash: String,
    cached: bool,
    staged: BTreeSet<String>,
}

pub struct Executor<'a> {
    spec: &'a RunSpec,
    backend: &'a dyn Backend,
    tasks: Vec<Task>,
    on_event: &'a (dyn Fn(&TaskEvent) + Sync),
}

impl<'a> Executor<'a> {
    pub fn new(spec: &'a RunSpec, backend: &'a dyn Backend, on_event: &'a (dyn Fn(&TaskEvent) + Sync)) -> Result<Self> {
        let mut ex = Executor {
            spec,
            backend,
            tasks: Vec::new(),
            on_event,
        };
        ex.build()?;
        Ok(ex)
    }

    fn build(&mut self) -> Result<()> {
        let plan = &self.spec.plan;
        let mut index: HashMap<(usize, Option<usize>), usize> = HashMap::new();
        for (si, step) in plan.steps.iter().enumerate() {
            if !step.runnable {
                continue;
            }
            let samples: Vec<Option<usize>> = if step.per_sample {
                (0..self.spec.samples.len()).map(Some).collect()
            } else {
                vec![None]
            };
            for sample in samples {
                let mut deps = Vec::new();
                let mut possible = true;
                for input in step.inputs.values() {
                    for source in picks(plan, input) {
                        let Source::Step(up, _) = source else { continue };
                        {
                            if step.per_sample {
                                // a per-sample input from an all-sample step never matches a sample
                                match index.get(&(up, sample)) {
                                    Some(&t) => deps.push(t),
                                    None => possible = false,
                                }
                            } else {
                                deps.extend(index.iter().filter(|((s, _), _)| *s == up).map(|(_, &t)| t));
                            }
                        }
                    }
                }
                // task order, so the hash and the staged files never depend on map order
                deps.sort_unstable();
                deps.dedup();
                let id = self.tasks.len();
                let tag = sample.map_or("all".to_string(), |s| self.spec.samples[s].sample.clone());
                self.tasks.push(Task {
                    step: si,
                    sample,
                    deps,
                    state: if possible { State::Pending } else { State::Skipped },
                    row: TraceRow {
                        task_id: id + 1,
                        hash: String::new(),
                        name: format!("STEP_{} ({tag})", step.id),
                        status: if possible { Status::Submitted } else { Status::Skipped },
                        exit: None,
                        submit: None,
                        start: None,
                        complete: None,
                        cpus: 0,
                        memory_gb: 0,
                        workdir: String::new(),
                    },
                });
                index.insert((si, sample), id);
            }
        }
        Ok(())
    }

    fn emit(&self, t: usize, status: Status, exit: Option<i32>, error: Option<String>) {
        let task = &self.tasks[t];
        (self.on_event)(&TaskEvent {
            step: self.spec.plan.steps[task.step].id.clone(),
            tag: task.tag(self.spec),
            status,
            hash: task.row.hash.clone(),
            cpus: task.row.cpus,
            memory_gb: task.row.memory_gb,
            exit,
            error,
        });
    }

    /// Ready: every dependency finished. Returns Some(false) when it must be skipped.
    fn readiness(&self, t: usize) -> Option<bool> {
        let task = &self.tasks[t];
        let mut skip = false;
        for &d in &task.deps {
            match &self.tasks[d].state {
                State::Pending | State::Running => return None,
                State::Skipped | State::Failed => skip |= task.sample.is_some(),
                State::Done(_) => {}
            }
        }
        Some(!skip)
    }

    /// Files for one input of a task, in staging order.
    fn input_files(&self, t: usize, sources: &[String]) -> Option<Vec<PathBuf>> {
        let task = &self.tasks[t];
        // no source runs (e.g. its step is off): an optional input with no files
        let Some(source) = pick(&self.spec.plan, sources) else {
            return Some(Vec::new());
        };
        match source {
            Source::Reads => {
                let s = |i: usize| {
                    // a pair of read files, or the one FASTA file of an assembly
                    let s = &self.spec.samples[i];
                    std::iter::once(s.r1.clone()).chain(s.r2.clone()).collect::<Vec<_>>()
                };
                Some(match task.sample {
                    Some(i) => s(i),
                    None => (0..self.spec.samples.len()).flat_map(s).collect(),
                })
            }
            Source::Step(up, out) => {
                let mut files = Vec::new();
                for &d in &task.deps {
                    let dep = &self.tasks[d];
                    if dep.step != up {
                        continue;
                    }
                    if let State::Done(outs) = &dep.state {
                        match outs.get(&out) {
                            Some(f) if !f.is_empty() => files.extend(f.iter().cloned()),
                            _ if task.sample.is_some() => return None,
                            _ => {}
                        }
                    }
                }
                Some(files)
            }
        }
    }

    fn prepare(&self, t: usize, cpus: u32, memory_gb: u32) -> Result<Option<Prepared>> {
        let spec = self.spec;
        let task = &self.tasks[t];
        let step = &spec.plan.steps[task.step];
        let tag = task.tag(spec);

        let mut env = vec![
            format!("export SAMPLE={} CPUS={cpus} MEM_GB={memory_gb}", sq(&tag)),
            format!("export TOOL_DIR=\"$PWD/tooldir\" STEP={}", sq(&step.id)),
            "mkdir -p out tooldir".to_string(),
            format!(
                "export SAMPLES={}",
                sq(&spec
                    .samples
                    .iter()
                    .map(|s| s.sample.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"))
            ),
        ];
        for (k, v) in &step.settings {
            env.push(format!("export S_{k}={}", sq(v)));
        }
        // (name in the task folder, file it points at)
        let mut staged: Vec<(String, PathBuf)> = Vec::new();
        let mut key = Sha256::new();
        key.update(serde_json::to_vec(step)?);
        key.update(format!("|{tag}|{cpus}|{memory_gb}|"));
        for (name, input) in &step.inputs {
            if input.collect {
                // every connected source that runs, each in its own folder <step>.<output>
                env.push(format!("mkdir -p in_{name}; export IN_{name}_DIR=\"$PWD/in_{name}\""));
                for src in &input.sources {
                    if pick(&spec.plan, std::slice::from_ref(src)).is_none() {
                        continue;
                    }
                    for f in self.input_files(t, std::slice::from_ref(src)).unwrap_or_default() {
                        let base = f.file_name().unwrap_or_default().to_string_lossy();
                        staged.push((format!("in_{name}/{src}/{base}"), f.clone()));
                    }
                }
                continue;
            }
            let Some(files) = self.input_files(t, &input.sources) else {
                return Ok(None); // an optional output this sample did not make
            };
            let base = |f: &PathBuf| f.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if !step.per_sample {
                env.push(format!("mkdir -p in_{name}; export IN_{name}_DIR=\"$PWD/in_{name}\""));
                for f in &files {
                    staged.push((format!("in_{name}/{}", base(f)), f.clone()));
                }
            } else if input.reads {
                let n: Vec<String> = files.iter().map(base).collect();
                if n.len() != 2 {
                    bail!(
                        "step {}: input {name} needs 2 read files for {tag}, got {}",
                        step.id,
                        n.len()
                    );
                }
                env.push(format!("export IN_{name}_1={} IN_{name}_2={}", sq(&n[0]), sq(&n[1])));
                staged.extend(n.into_iter().zip(files.iter().cloned()));
            } else if !files.is_empty() || !input.optional {
                let n: Vec<String> = files.iter().map(base).collect();
                env.push(format!("export IN_{name}={}", sq(&n.join(" "))));
                staged.extend(n.into_iter().zip(files.iter().cloned()));
            }
        }
        let mut seen = BTreeSet::new();
        for (n, f) in &staged {
            if !seen.insert(n.clone()) {
                bail!("step {}: two input files called {n} for {tag}", step.id);
            }
            key.update(file_id(f));
        }
        let mut mounts: Vec<(PathBuf, bool)> = vec![(spec.work_dir.clone(), false)];
        for d in &step.databases {
            let dir = spec.db_dir.join(&spec.plan.databases[d]);
            if !dir.is_dir() {
                bail!("step {}: database {d} is not installed ({})", step.id, dir.display());
            }
            env.push(format!("export DB_{d}={}", sq(&dir.to_string_lossy())));
            key.update(dir.to_string_lossy().as_bytes());
            mounts.push((dir, true));
        }
        // a database the step picked: the tool's command still asks for its own ($DB_<own>)
        for (own, picked) in &step.db_alias {
            env.push(format!("export DB_{own}=\"$DB_{picked}\""));
        }
        // what the user set up for this tool (a database of their own); it may still be empty
        if let Some(rel) = &step.user_data {
            let dir = spec.db_dir.join(rel);
            fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
            env.push(format!("export USER_DATA={}", sq(&dir.to_string_lossy())));
            key.update(user_data_id(&dir));
            mounts.push((dir, true));
        }
        for (_, f) in &staged {
            if let Some(parent) = f.parent() {
                if !parent.starts_with(&spec.work_dir) && !mounts.iter().any(|(m, _)| m == parent) {
                    mounts.push((parent.to_path_buf(), true));
                }
            }
        }
        let bundle = spec.run_dir.join("tools").join(&step.tool_bundle);
        for f in &step.files {
            for e in walkdir::WalkDir::new(bundle.join(f))
                .follow_links(true)
                .into_iter()
                .flatten()
            {
                key.update(file_id(e.path()));
            }
        }

        let hash = format!("{:x}", key.finalize());
        let dir = spec.work_dir.join(&hash[..2]).join(&hash[2..32]);
        let name = format!("gwm-{}-{}", spec.run_id, &hash[..12])
            .replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "-");
        let run = TaskRun {
            name,
            dir: dir.clone(),
            image: step.container.clone(),
            cpus,
            memory_gb,
            mounts,
        };
        let staged_names: BTreeSet<String> = staged.iter().map(|(n, _)| n.clone()).collect();
        if dir.join(".gwm_ok").is_file() {
            return Ok(Some(Prepared {
                run,
                hash,
                cached: true,
                staged: staged_names,
            }));
        }

        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(dir.join("tooldir"))?;
        for (n, f) in &staged {
            let at = dir.join(n);
            if let Some(p) = at.parent() {
                fs::create_dir_all(p)?;
            }
            link(f, &at).with_context(|| format!("staging {}", f.display()))?;
        }
        for f in &step.files {
            copy_tree(&bundle.join(f), &dir.join("tooldir").join(f))?;
        }
        let mut versions: Vec<String> = step
            .versions
            .iter()
            .map(|(k, cmd)| {
                format!(
                    "printf '%s: %s\\n' {} \"$( ({cmd}) 2>&1 | head -1 )\" >> versions.yml",
                    sq(k)
                )
            })
            .collect();
        versions.push(format!(
            "echo {} >> versions.yml",
            sq(&format!("container.STEP_{}: {}", step.id, step.container))
        ));
        versions.push(format!(
            "[ -f .ntg_notes ] && sed 's/^/note.{}./' .ntg_notes >> versions.yml || true",
            step.id
        ));
        let script = format!(
            "#!/bin/bash -ue\n# {}: {} {}\n{}\n\n{}\n\n{}\n",
            step.label,
            step.tool_name,
            step.tool_version,
            env.join("\n"),
            step.command.trim_end(),
            versions.join("\n")
        );
        fs::write(dir.join(".command.sh"), script)?;
        Ok(Some(Prepared {
            run,
            hash,
            cached: false,
            staged: staged_names,
        }))
    }

    /// Output files of a finished task, by output name. Error: a required output is missing.
    fn collect(&self, t: usize, dir: &Path, staged: &BTreeSet<String>) -> Result<IndexMap<String, Vec<PathBuf>>> {
        let task = &self.tasks[t];
        let step = &self.spec.plan.steps[task.step];
        let tag = task.tag(self.spec);
        let mut files: Vec<(String, PathBuf)> = Vec::new();
        for e in walkdir::WalkDir::new(dir)
            .min_depth(1)
            .follow_links(false)
            .sort_by_file_name()
        {
            let e = e?;
            let rel = e.path().strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
            if rel.split('/').any(|p| p.starts_with('.')) || rel == "tooldir" || rel.starts_with("tooldir/") {
                continue;
            }
            if staged.contains(&rel) || staged.iter().any(|s| rel.starts_with(&format!("{s}/"))) {
                continue;
            }
            files.push((rel, e.path().to_path_buf()));
        }
        let mut out = IndexMap::new();
        for (name, o) in &step.outputs {
            let pattern = o.path.replace("{sample}", &tag);
            let glob = GlobBuilder::new(&pattern)
                .literal_separator(true)
                .build()?
                .compile_matcher();
            let found: Vec<PathBuf> = files
                .iter()
                .filter(|(r, _)| glob.is_match(r))
                .map(|(_, p)| p.clone())
                .collect();
            if found.is_empty() && !o.optional {
                bail!("step {} ({tag}): missing output file(s) `{pattern}`", step.id);
            }
            out.insert(name.clone(), found);
        }
        Ok(out)
    }

    /// A step whose tool writes reports: the reports are also written as tables
    /// (`out/tables/`, the tool's `tables` output), then the outputs are listed
    /// again with those in.
    fn with_report_tables(
        &self,
        t: usize,
        dir: &Path,
        staged: &BTreeSet<String>,
        outs: IndexMap<String, Vec<PathBuf>>,
    ) -> Result<IndexMap<String, Vec<PathBuf>>> {
        let step = &self.spec.plan.steps[self.tasks[t].step];
        if step.report_outputs.is_empty() {
            return Ok(outs);
        }
        // a report the app can't read as tables never stops a run: the step keeps its own outputs
        let tag = self.tasks[t].tag(self.spec);
        let written = std::panic::catch_unwind(|| crate::reports::write_tables(dir, &outs, &step.report_outputs, &tag));
        match written {
            Ok(Ok(_)) => self.collect(t, dir, staged),
            Ok(Err(e)) => {
                eprintln!("{}: its report was not turned into tables ({e:#})", step.id);
                Ok(outs)
            }
            Err(_) => {
                eprintln!("{}: its report was not turned into tables", step.id);
                Ok(outs)
            }
        }
    }

    fn publish(&self, t: usize, dir: &Path, outputs: &IndexMap<String, Vec<PathBuf>>) -> Result<()> {
        let task = &self.tasks[t];
        let step = &self.spec.plan.steps[task.step];
        let dest = self
            .spec
            .results_dir
            .join(step.publish.replace("{sample}", &task.tag(self.spec)));
        for f in outputs.values().flatten() {
            let rel = f.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
            if let Some(rel) = rel.strip_prefix("out/") {
                copy_tree(f, &dest.join(rel))?;
            }
        }
        Ok(())
    }

    fn task_resources(&self, step: &PlanStep) -> (u32, u32) {
        let l = &self.spec.limits;
        let cpus = step.resources.cpus.unwrap_or(1).clamp(1, l.cpus.max(1));
        let mem = task_memory_gb(step, l.total_memory_gb).clamp(1, l.memory_gb.max(1));
        (cpus, mem)
    }

    fn error_tail(dir: &Path) -> String {
        let read = |f: &str| fs::read_to_string(dir.join(f)).unwrap_or_default();
        let err = read(".command.err");
        let text = if err.trim().is_empty() {
            read(".command.out")
        } else {
            err
        };
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(20)..].join("\n")
    }

    /// Run every task. `stop` set from outside = stop the run (tasks are killed).
    pub fn run(mut self, stop: &AtomicBool) -> Result<Outcome> {
        fs::create_dir_all(&self.spec.work_dir)?;
        fs::create_dir_all(&self.spec.results_dir)?;
        let kill = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel::<(usize, Result<i32>)>();
        let mut error: Option<String> = None;
        let mut stopped = false;
        let (mut used_cpus, mut used_mem) = (0u32, 0u32);
        let mut running: HashMap<usize, (Prepared, Instant)> = HashMap::new();
        let mut versions: BTreeSet<String> = BTreeSet::new();

        std::thread::scope(|scope| -> Result<()> {
            loop {
                if stop.load(Ordering::Relaxed) && !stopped {
                    stopped = true;
                    kill.store(true, Ordering::Relaxed);
                }
                // start what is ready and fits
                let mut progressed = true;
                while progressed && error.is_none() && !stopped {
                    progressed = false;
                    for t in 0..self.tasks.len() {
                        if !matches!(self.tasks[t].state, State::Pending) {
                            continue;
                        }
                        let Some(ok) = self.readiness(t) else { continue };
                        if !ok {
                            self.tasks[t].state = State::Skipped;
                            self.tasks[t].row.status = Status::Skipped;
                            self.emit(t, Status::Skipped, None, None);
                            progressed = true;
                            continue;
                        }
                        let step = &self.spec.plan.steps[self.tasks[t].step];
                        let (cpus, mem) = self.task_resources(step);
                        if used_cpus + cpus > self.spec.limits.cpus.max(cpus)
                            || used_mem + mem > self.spec.limits.memory_gb.max(mem)
                        {
                            continue;
                        }
                        self.tasks[t].row.cpus = cpus;
                        self.tasks[t].row.memory_gb = mem;
                        self.tasks[t].row.submit = Some(now());
                        let prepared = match self.prepare(t, cpus, mem) {
                            Ok(Some(p)) => p,
                            Ok(None) => {
                                self.tasks[t].state = State::Skipped;
                                self.tasks[t].row.status = Status::Skipped;
                                self.emit(t, Status::Skipped, None, None);
                                progressed = true;
                                continue;
                            }
                            Err(e) => {
                                error = Some(format!("{e:#}"));
                                self.tasks[t].state = State::Failed;
                                self.tasks[t].row.status = Status::Failed;
                                self.emit(t, Status::Failed, None, error.clone());
                                break;
                            }
                        };
                        progressed = true;
                        self.tasks[t].row.hash = format!("{}/{}", &prepared.hash[..2], &prepared.hash[2..8]);
                        self.tasks[t].row.workdir = prepared.run.dir.to_string_lossy().into_owned();
                        if prepared.cached {
                            match self.collect(t, &prepared.run.dir, &prepared.staged) {
                                Ok(outs) => {
                                    self.publish(t, &prepared.run.dir, &outs)?;
                                    read_versions(&prepared.run.dir, &mut versions);
                                    self.tasks[t].state = State::Done(outs);
                                    self.tasks[t].row.status = Status::Cached;
                                    self.tasks[t].row.exit = Some(0);
                                    self.emit(t, Status::Cached, Some(0), None);
                                    continue;
                                }
                                Err(_) => {
                                    // outputs gone: run it again
                                    fs::remove_file(prepared.run.dir.join(".gwm_ok")).ok();
                                    match self.prepare(t, cpus, mem)? {
                                        Some(p) => {
                                            self.tasks[t].row.start = Some(now());
                                            self.tasks[t].state = State::Running;
                                            self.launch(scope, t, p, &tx, &kill, &mut running)
                                        }
                                        None => continue,
                                    }
                                }
                            }
                        } else {
                            self.tasks[t].row.start = Some(now());
                            self.tasks[t].state = State::Running;
                            self.launch(scope, t, prepared, &tx, &kill, &mut running);
                        }
                        used_cpus += cpus;
                        used_mem += mem;
                    }
                }
                if running.is_empty() {
                    break;
                }
                let (t, result) = match rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(m) => m,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(e) => return Err(anyhow!(e)),
                };
                let (prepared, _) = running.remove(&t).unwrap();
                used_cpus -= self.tasks[t].row.cpus;
                used_mem -= self.tasks[t].row.memory_gb;
                self.tasks[t].row.complete = Some(now());
                let dir = prepared.run.dir.clone();
                let code = match result {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = fs::write(dir.join(".command.err"), format!("{e:#}"));
                        -1
                    }
                };
                fs::write(dir.join(".exitcode"), code.to_string()).ok();
                self.tasks[t].row.exit = Some(code);
                if kill.load(Ordering::Relaxed) {
                    self.tasks[t].state = State::Failed;
                    self.tasks[t].row.status = Status::Aborted;
                    self.emit(t, Status::Aborted, Some(code), None);
                    continue;
                }
                let collected = if code == 0 {
                    self.collect(t, &dir, &prepared.staged)
                        .and_then(|outs| self.with_report_tables(t, &dir, &prepared.staged, outs))
                } else {
                    Err(anyhow!("exit status {code}"))
                };
                match collected {
                    Ok(outs) => {
                        self.publish(t, &dir, &outs)?;
                        read_versions(&dir, &mut versions);
                        fs::write(dir.join(".gwm_ok"), "").ok();
                        self.tasks[t].state = State::Done(outs);
                        self.tasks[t].row.status = Status::Completed;
                        self.emit(t, Status::Completed, Some(code), None);
                    }
                    Err(e) => {
                        let msg = format!(
                            "{} failed: {e:#}\nWork folder: {}\n{}",
                            self.tasks[t].row.name,
                            dir.display(),
                            Self::error_tail(&dir)
                        );
                        self.tasks[t].state = State::Failed;
                        self.tasks[t].row.status = Status::Failed;
                        self.emit(t, Status::Failed, Some(code), Some(msg.clone()));
                        if error.is_none() {
                            error = Some(msg);
                        }
                        kill.store(true, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        })?;

        let mut ran: Vec<String> = Vec::new();
        for t in &self.tasks {
            let id = &self.spec.plan.steps[t.step].id;
            if matches!(t.state, State::Done(_)) && !ran.contains(id) {
                ran.push(id.clone());
            }
        }
        let success = error.is_none() && !stopped;
        Ok(Outcome {
            success,
            stopped,
            error,
            trace: self.tasks.iter().map(|t| t.row.clone()).collect(),
            versions: versions.into_iter().collect(),
            ran,
        })
    }

    fn launch<'s>(
        &self,
        scope: &'s std::thread::Scope<'s, '_>,
        t: usize,
        p: Prepared,
        tx: &mpsc::Sender<(usize, Result<i32>)>,
        kill: &'s AtomicBool,
        running: &mut HashMap<usize, (Prepared, Instant)>,
    ) where
        'a: 's,
    {
        let tx = tx.clone();
        let backend = self.backend;
        let run = p.run.clone();
        scope.spawn(move || {
            let _ = tx.send((t, backend.run(&run, kill)));
        });
        self.emit(t, Status::Running, None, None);
        running.insert(t, (p, Instant::now()));
    }
}

fn read_versions(dir: &Path, into: &mut BTreeSet<String>) {
    if let Ok(text) = fs::read_to_string(dir.join("versions.yml")) {
        into.extend(
            text.lines()
                .map(str::trim)
                .filter(|l| l.contains(':'))
                .map(String::from),
        );
    }
}

/// Seconds since the Unix epoch (for file stamps).
pub fn epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
