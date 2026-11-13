//! A problem report for a run that failed or stopped: one text file with what
//! is needed to find out why, to save and send to the people who look after
//! the app. It holds no sequence data, and sample names and the computer's
//! user name are replaced (sample_1, <user>) before it is written.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Local;
use gwm_core::Catalog;
use serde_json::Value;

use crate::runs::{load, run_path, work_name, DETAILS, RESULTS};

const MAX_REPORT: usize = 1_500_000;

pub fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let from = all.len().saturating_sub(lines);
    let mut out = String::new();
    if from > 0 {
        let _ = writeln!(out, "... ({from} earlier lines left out)");
    }
    out.push_str(&all[from..].join("\n"));
    out
}

pub fn head(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let mut out = all[..all.len().min(lines)].join("\n");
    if all.len() > lines {
        let _ = write!(out, "\n... ({} more lines left out)", all.len() - lines);
    }
    out
}

pub fn read(path: &Path) -> String {
    fs::read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

pub fn section(out: &mut String, title: &str, body: &str) {
    let body = body.trim_end();
    let _ = writeln!(
        out,
        "\n===== {title} =====\n{}",
        if body.is_empty() { "(nothing)" } else { body }
    );
}

/// The user's name in paths (`/Users/<name>/`, `\Users\<name>\`, `/home/<name>/`) as `<user>`.
pub fn mask_users(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'outer: loop {
        let mut next: Option<(usize, &str, char)> = None;
        for (mark, sep) in [("/Users/", '/'), ("\\Users\\", '\\'), ("/home/", '/')] {
            if let Some(i) = rest.find(mark) {
                if next.is_none_or(|(j, _, _)| i < j) {
                    next = Some((i, mark, sep));
                }
            }
        }
        let Some((i, mark, sep)) = next else { break 'outer };
        let after = i + mark.len();
        out.push_str(&rest[..after]);
        let name_end = rest[after..]
            .find([sep, '"', '\'', '\n', '\r', '\t'])
            .map_or(rest.len(), |e| after + e);
        if name_end > after {
            out.push_str("<user>");
        }
        rest = &rest[name_end..];
    }
    out.push_str(rest);
    out
}

/// A task's work folder, newest first, for the tasks that did not finish.
fn unfinished_tasks(work: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for a in fs::read_dir(work).into_iter().flatten().flatten() {
        for b in fs::read_dir(a.path()).into_iter().flatten().flatten() {
            let dir = b.path();
            if dir.join(".command.sh").is_file() && !dir.join(".gwm_ok").exists() {
                let when = b.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                dirs.push((when, dir));
            }
        }
    }
    dirs.sort_by_key(|a| std::cmp::Reverse(a.0));
    dirs.into_iter().take(6).map(|(_, d)| d).collect()
}

pub fn listing(dir: &Path) -> String {
    let mut out = String::new();
    for e in walkdir::WalkDir::new(dir)
        .min_depth(1)
        .max_depth(4)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .flatten()
        .take(120)
    {
        let rel = e
            .path()
            .strip_prefix(dir)
            .unwrap_or(e.path())
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with("tooldir") {
            continue;
        }
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        let _ = writeln!(
            out,
            "{:>12}  {rel}{}",
            size,
            if e.file_type().is_dir() { "/" } else { "" }
        );
    }
    out
}

/// The report. `computer`: the app's check of this computer (Settings page).
pub fn problem_report(cat: &Catalog, run_id: &str, computer: &Value) -> Result<String> {
    let run = load(cat, run_id)?;
    let dir = run_path(cat, run_id)?;
    let details = dir.join(DETAILS);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Genomics Workflow Microbials: problem report\n\
         Made {}\n\n\
         This file is for the people who look after the app. It holds what the run was asked to do, what each\n\
         step printed and where it stopped. It holds no sequence data. Sample names are replaced by sample_1,\n\
         sample_2 ... and the computer's user name by <user>. Please read it through before sending it.",
        Local::now().format("%Y-%m-%d %H:%M")
    );

    let text = |v: &Value| v.as_str().unwrap_or("").to_string();
    section(
        &mut out,
        "The app and this computer",
        &format!(
            "App version: {}\nSystem: {}{}\nContainers: Docker {} / Apptainer {}{}\nCPU threads: {}\nMemory: {} GB ({} GB free)\nFree disk: working files {} GB, databases {} GB",
            env!("CARGO_PKG_VERSION"),
            text(&computer["os"]),
            if computer["wsl"] == true { " (inside WSL)" } else { "" },
            computer["docker"].as_str().unwrap_or("not found"),
            computer["apptainer"].as_str().unwrap_or("not found"),
            computer["docker_error"]
                .as_str()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default(),
            computer["cpus"],
            computer["memory_gb"],
            computer["memory_available_gb"],
            computer["disk_free_gb"]["work"],
            computer["disk_free_gb"]["databases"],
        ),
    );

    section(
        &mut out,
        "The run",
        &format!(
            "Name: {}\nStatus: {}\nWorkflow: {} version {}\nStarted: {}\nFinished: {}\nSamples: {}\nSteps switched off: {}\nReads folder: {}\nLimits: {}\n\nWhat the app said went wrong:\n{}",
            text(&run["name"]),
            text(&run["status"]),
            text(&run["workflow"]["id"]),
            run["workflow"]["version"],
            text(&run["started"]),
            text(&run["finished"]),
            run["n_samples"],
            run["steps_off"],
            text(&run["input"]),
            run["limits"],
            run["error"].as_str().unwrap_or("(nothing recorded)"),
        ),
    );

    // each step and each sample's state
    let mut states = String::new();
    if let Some(steps) = run["steps"].as_object() {
        for (step, s) in steps {
            for (tag, t) in s["tasks"].as_object().into_iter().flatten() {
                let _ = writeln!(
                    states,
                    "{step:<24} {tag:<28} {:<10} exit {}  {} -> {}",
                    text(&t["status"]),
                    t["exit"],
                    text(&t["started_at"]),
                    text(&t["finished_at"])
                );
            }
        }
    }
    section(&mut out, "Steps and samples", &states);

    if let Ok(plan) = serde_json::from_str::<Value>(&read(&details.join("plan.json"))) {
        let mut steps = String::new();
        for s in plan["steps"].as_array().into_iter().flatten() {
            let _ = writeln!(
                steps,
                "{:<24} {} {}  container {}  runs: {}  databases: {}",
                text(&s["id"]),
                text(&s["tool_name"]),
                text(&s["tool_version"]),
                text(&s["container"]),
                s["runnable"],
                s["databases"]
            );
        }
        section(&mut out, "The plan (tools, versions, containers)", &steps);
    }
    section(
        &mut out,
        "The workflow as it ran",
        &head(&read(&details.join("workflow.yml")), 400),
    );
    section(
        &mut out,
        "The run's own log (the end of it)",
        &tail(&read(&details.join("exec.log")), 300),
    );

    // the steps that did not finish: their script, what they printed, what they left
    let work = cat.home.join("work").join(work_name(run_id));
    let tasks = unfinished_tasks(&work);
    if tasks.is_empty() {
        section(
            &mut out,
            "Steps that did not finish",
            "No unfinished step was found in the working files (they may have been cleared).",
        );
    }
    for (i, task) in tasks.iter().enumerate() {
        let name = format!("Unfinished step {} of {}", i + 1, tasks.len());
        section(
            &mut out,
            &format!("{name}: its script"),
            &head(&read(&task.join(".command.sh")), 200),
        );
        section(&mut out, &format!("{name}: exit code"), &read(&task.join(".exitcode")));
        section(
            &mut out,
            &format!("{name}: errors it printed (the end)"),
            &tail(&read(&task.join(".command.err")), 200),
        );
        section(
            &mut out,
            &format!("{name}: other output (the end)"),
            &tail(&read(&task.join(".command.out")), 80),
        );
        section(
            &mut out,
            &format!("{name}: files in its folder (size, name)"),
            &listing(task),
        );
    }
    section(
        &mut out,
        "Tool versions recorded",
        &head(&read(&dir.join(RESULTS).join("pipeline_info/tool_versions.yml")), 200),
    );

    // names out: the samples (longest first, so one name inside another is not cut), then the user
    let mut samples: Vec<String> = read(&details.join("samples.csv"))
        .lines()
        .skip(1)
        .filter_map(|l| l.split(',').next().map(str::to_string))
        .filter(|s| s.len() >= 3)
        .collect();
    let order = samples.clone();
    samples.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for s in &samples {
        let n = order.iter().position(|o| o == s).unwrap_or(0) + 1;
        out = out.replace(s.as_str(), &format!("sample_{n}"));
    }
    let mut out = mask_users(&out);
    if out.len() > MAX_REPORT {
        let mut cut = MAX_REPORT;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("\n... (the report was cut here: it was too long)\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_names_are_left_out() {
        assert_eq!(
            mask_users("reads in /mnt/c/Users/Jo Smith/Desktop/x and C:\\Users\\Jo Smith\\y, /home/jo/z"),
            "reads in /mnt/c/Users/<user>/Desktop/x and C:\\Users\\<user>\\y, /home/<user>/z"
        );
        assert_eq!(mask_users("no paths here"), "no paths here");
    }

    #[test]
    fn long_text_is_shortened() {
        let text = (1..=10).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        assert_eq!(tail(&text, 2), "... (8 earlier lines left out)\n9\n10");
        assert_eq!(head(&text, 2), "1\n2\n... (8 more lines left out)");
    }
}
