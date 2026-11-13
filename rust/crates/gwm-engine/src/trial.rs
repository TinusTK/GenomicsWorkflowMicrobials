//! Try a tool once, outside a run: the tool editor's "Show the tool's help" and
//! "Try on one sample". The script is built the same way a run builds it
//! (same variables, same folders), in a folder that is emptied each time.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{bail, Context, Result};
use globset::GlobBuilder;
use gwm_core::{value, Tool};
use indexmap::IndexMap;
use serde::Serialize;

use crate::backend::{Backend, TaskRun};

pub struct Trial<'a> {
    pub tool: &'a Tool,
    /// run this instead of the tool's command (e.g. `mytool --help`)
    pub command: Option<String>,
    pub sample: String,
    /// input -> its files (reads: the pair)
    pub inputs: IndexMap<String, Vec<PathBuf>>,
    /// emptied, then used as the task folder
    pub dir: PathBuf,
    pub db_dir: PathBuf,
    /// database id -> folder name under `db_dir`
    pub databases: IndexMap<String, String>,
    pub cpus: u32,
    pub memory_gb: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrialOutput {
    pub name: String,
    pub pattern: String,
    pub optional: bool,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrialFile {
    pub path: String,
    pub size: u64,
    /// the first lines of a small text file
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrialResult {
    pub exit: i32,
    /// what the command printed (the end of it)
    pub log: String,
    /// each output of the tool and the files that matched it
    pub outputs: Vec<TrialOutput>,
    /// every file the command made
    pub files: Vec<TrialFile>,
    pub dir: PathBuf,
    /// the sample it ran on ("all" for an all-sample tool)
    pub sample: String,
}

fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(unix)]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}

#[cfg(not(unix))]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    fs::copy(target, at).map(|_| ())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
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
    Ok(())
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn preview(path: &Path, size: u64) -> Option<String> {
    use std::io::Read;
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u8; 16_384.min(size as usize)];
    let n = fs::File::open(path).ok()?.read(&mut buf).ok()?;
    let text = std::str::from_utf8(&buf[..n]).ok().or_else(|| {
        // cut in the middle of a character: drop the last bytes
        (1..4).find_map(|k| std::str::from_utf8(&buf[..n.saturating_sub(k)]).ok())
    })?;
    if text.contains('\0') {
        return None;
    }
    Some(text.lines().take(30).collect::<Vec<_>>().join("\n"))
}

pub fn run(t: &Trial, backend: &dyn Backend, kill: &AtomicBool) -> Result<TrialResult> {
    let tool = t.tool;
    if tool.container.trim().is_empty() && backend.name() != "local" {
        bail!("this tool has no container yet: fill in its container address first");
    }
    if t.dir.exists() {
        fs::remove_dir_all(&t.dir)?;
    }
    fs::create_dir_all(t.dir.join("tooldir"))?;

    let mut env = vec![
        format!("export SAMPLE={} CPUS={} MEM_GB={}", sq(&t.sample), t.cpus, t.memory_gb),
        format!("export TOOL_DIR=\"$PWD/tooldir\" STEP={}", sq(&tool.id)),
        "mkdir -p out tooldir".to_string(),
        format!("export SAMPLES={}", sq(&t.sample)),
    ];
    for (k, spec) in &tool.settings {
        env.push(format!("export S_{k}={}", sq(&value::to_text(&spec.default))));
    }
    let mut mounts: Vec<(PathBuf, bool)> = vec![(t.dir.clone(), false)];
    let mut staged: Vec<String> = Vec::new();
    // a stand-in command (the help text) reads no inputs
    for (name, spec) in tool.inputs.iter().filter(|_| t.command.is_none()) {
        let files = t.inputs.get(name).cloned().unwrap_or_default();
        let base = |f: &PathBuf| f.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let mut stage = |at: String, f: &PathBuf| -> Result<()> {
            let dest = t.dir.join(&at);
            if let Some(p) = dest.parent() {
                fs::create_dir_all(p)?;
            }
            link(f, &dest).with_context(|| format!("staging {}", f.display()))?;
            if let Some(parent) = f.parent() {
                if !mounts.iter().any(|(m, _)| m == parent) {
                    mounts.push((parent.to_path_buf(), true));
                }
            }
            staged.push(at);
            Ok(())
        };
        if !tool.per_sample() {
            env.push(format!("mkdir -p in_{name}; export IN_{name}_DIR=\"$PWD/in_{name}\""));
            for f in &files {
                stage(format!("in_{name}/{}", base(f)), f)?;
            }
        } else if spec.kind == "reads" {
            if files.len() != 2 {
                bail!("input {name} needs a pair of read files");
            }
            env.push(format!(
                "export IN_{name}_1={} IN_{name}_2={}",
                sq(&base(&files[0])),
                sq(&base(&files[1]))
            ));
            for f in &files {
                stage(base(f), f)?;
            }
        } else if !files.is_empty() {
            let names: Vec<String> = files.iter().map(base).collect();
            env.push(format!("export IN_{name}={}", sq(&names.join(" "))));
            for f in &files {
                stage(base(f), f)?;
            }
        } else if !spec.optional {
            bail!("input {name} needs a file");
        }
    }
    for d in &tool.databases {
        let Some(folder) = t.databases.get(d) else {
            bail!("database {d} is not in the databases list");
        };
        let dir = t.db_dir.join(folder);
        if !dir.is_dir() {
            bail!("database {d} is not installed (Databases page)");
        }
        env.push(format!("export DB_{d}={}", sq(&dir.to_string_lossy())));
        mounts.push((dir, true));
    }
    // what the user set up for this tool (a database of their own), as a run gives it
    if let Some(rel) = gwm_core::compiler::user_data(tool) {
        let dir = t.db_dir.join(rel);
        fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
        env.push(format!("export USER_DATA={}", sq(&dir.to_string_lossy())));
        mounts.push((dir, true));
    }
    for f in &tool.files {
        let from = tool.dir.join(f);
        if from.is_dir() {
            copy_tree(&from, &t.dir.join("tooldir").join(f))?;
        } else if from.is_file() {
            let dest = t.dir.join("tooldir").join(f);
            if let Some(p) = dest.parent() {
                fs::create_dir_all(p)?;
            }
            fs::copy(&from, dest)?;
        }
    }
    let command = t.command.clone().unwrap_or_else(|| tool.command.clone());
    fs::write(
        t.dir.join(".command.sh"),
        format!("#!/bin/bash -ue\n{}\n\n{}\n", env.join("\n"), command.trim_end()),
    )?;

    let task = TaskRun {
        name: format!("gwm-try-{}", tool.id.replace('_', "-")),
        dir: t.dir.clone(),
        image: tool.container.clone(),
        cpus: t.cpus,
        memory_gb: t.memory_gb,
        mounts,
    };
    let exit = backend.run(&task, kill)?;

    let read = |f: &str| fs::read_to_string(t.dir.join(f)).unwrap_or_default();
    let (out, err) = (read(".command.out"), read(".command.err"));
    let log = match (out.trim().is_empty(), err.trim().is_empty()) {
        (false, false) => format!("{}\n{}", tail(&err, 60), tail(&out, 300)),
        (true, _) => tail(&err, 300),
        (_, true) => tail(&out, 300),
    };

    let mut made: Vec<(String, PathBuf, u64)> = Vec::new();
    for e in walkdir::WalkDir::new(&t.dir)
        .min_depth(1)
        .follow_links(false)
        .sort_by_file_name()
    {
        let e = e?;
        if !e.file_type().is_file() {
            continue;
        }
        let rel = e.path().strip_prefix(&t.dir)?.to_string_lossy().replace('\\', "/");
        if rel.split('/').any(|p| p.starts_with('.')) || rel.starts_with("tooldir/") || staged.contains(&rel) {
            continue;
        }
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        made.push((rel, e.path().to_path_buf(), size));
    }
    let mut outputs = Vec::new();
    for (name, o) in &tool.outputs {
        let pattern = o.path.replace("{sample}", &t.sample);
        let files = match GlobBuilder::new(&pattern).literal_separator(true).build() {
            Ok(g) => {
                let g = g.compile_matcher();
                made.iter()
                    .filter(|(r, _, _)| g.is_match(r))
                    .map(|(r, _, _)| r.clone())
                    .collect()
            }
            Err(_) => Vec::new(),
        };
        outputs.push(TrialOutput {
            name: name.clone(),
            pattern,
            optional: o.optional,
            files,
        });
    }
    let files = made
        .iter()
        .take(200)
        .map(|(rel, path, size)| TrialFile {
            path: rel.clone(),
            size: *size,
            preview: preview(path, *size),
        })
        .collect();
    Ok(TrialResult {
        exit,
        log,
        outputs,
        files,
        dir: t.dir.clone(),
        sample: t.sample.clone(),
    })
}
