//! A tool report: one text file for when adding or setting up a tool does
//! not work, to save and send to the people who look after the app (a GitHub
//! issue or an email). It holds the tool's definition, what the app said
//! while it built, set up and tried the tool, and what the last try left.
//!
//! Nothing private goes in: no sequence data and no file contents other than
//! the tool's own script and messages; the names of the files it was tried
//! on, the computer's user name, email addresses and anything that looks like
//! a key, token or password are replaced before the report is written.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{anyhow, Result};
use chrono::Local;
use gwm_core::Catalog;
use gwm_engine::diagnose::{head, listing, mask_users, read, section, tail};
use regex::Regex;
use serde_json::Value;

const MAX_REPORT: usize = 1_000_000;
/// What the app does with a tool, in the order it happens.
const KINDS: [(&str, &str); 4] = [
    ("build", "Building its container"),
    ("setup", "Set it up for me"),
    ("try", "Try on one sample"),
    ("help", "Show the tool's help"),
];

fn log_dir(cat: &Catalog, id: &str) -> PathBuf {
    cat.home.join("tool_logs").join(id)
}

/// A tool id is a plain name: never a way out of the logs folder.
fn plain(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('.') && id.chars().all(|c| c.is_ascii_alphanumeric() || "_-.@+".contains(c))
}

/// Keep what a tool job said, for a later report (the newest of each kind).
/// Never fails the job.
pub fn keep_log(cat: &Catalog, id: &str, kind: &str, lines: &[String], error: Option<&anyhow::Error>) {
    let dir = log_dir(cat, id);
    if !plain(id) || fs::create_dir_all(&dir).is_err() {
        return;
    }
    let mut text = format!(
        "{}  {}\n",
        Local::now().format("%Y-%m-%d %H:%M"),
        if error.is_some() { "FAILED" } else { "finished" }
    );
    for l in lines {
        text.push_str(l);
        text.push('\n');
    }
    if let Some(e) = error {
        let said = format!("Error: {e:#}");
        if lines.last() != Some(&said) {
            text.push_str(&said);
            text.push('\n');
        }
    }
    let _ = fs::write(dir.join(format!("{kind}.txt")), text);
}

/// Forget a removed tool's logs.
pub fn forget(cat: &Catalog, id: &str) {
    if plain(id) {
        let _ = fs::remove_dir_all(log_dir(cat, id));
    }
}

// ---------------------------------------------------------------- nothing private

static SECRETS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        // name=value / name: value, where the name says it is a secret
        (
            r#"(?i)\b([a-z0-9_.-]*(?:key|token|secret|passw(?:or)?d|passwd|credential)[a-z0-9_.-]*)(\s*[=:]\s*)("[^"\n]*"|'[^'\n]*'|[^\s"']+)"#,
            "$1$2<hidden>",
        ),
        // a login inside a link
        (r"://[^/\s:@]+:[^/\s@]+@", "://<hidden>@"),
        (r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}", "$1 <hidden>"),
        // tokens by their look (GitHub, and long random strings after a header name)
        (r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b", "<hidden>"),
        (r"(?i)\b(x-api-key|authorization)\b[^\n]*", "$1 <hidden>"),
        (r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", "<email>"),
    ]
    .into_iter()
    .map(|(re, to)| (Regex::new(re).expect("a fixed pattern"), to))
    .collect()
});

/// Keys, tokens, passwords, logins in links and email addresses, replaced.
fn mask_secrets(text: &str) -> String {
    let mut out = text.to_string();
    for (re, to) in SECRETS.iter() {
        out = re.replace_all(&out, *to).into_owned();
    }
    out
}

/// The names a try-out's folder gives away: the sample's name and the files
/// it was tried on (reads, an assembly), without their endings.
fn private_names(try_dir: &Path) -> Vec<String> {
    static SAMPLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"export SAMPLE='([^']*)'").expect("a fixed pattern"));
    static READ_END: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)([._-]R?[12])?([._-]001)?$").expect("a fixed pattern"));
    let mut names = Vec::new();
    if let Some(c) = SAMPLE.captures(&read(&try_dir.join(".command.sh"))) {
        names.push(c[1].to_string());
    }
    // the input files are links (or copies) at the top of the folder
    for e in fs::read_dir(try_dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let staged = e.file_type().is_ok_and(|t| t.is_symlink());
        if name.starts_with('.') || !staged {
            continue;
        }
        let stem = name.split('.').next().unwrap_or(&name);
        names.push(READ_END.replace(stem, "").into_owned());
        names.push(stem.to_string());
    }
    // "sample" and "all" are the app's own stand-in names, and short names would hit other words
    names.retain(|n| n.len() >= 3 && n != "sample" && n != "all");
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    names.dedup();
    names
}

fn clean(text: &str, names: &[String]) -> String {
    let mut out = text.to_string();
    for (i, n) in names.iter().enumerate() {
        out = out.replace(n.as_str(), &format!("tried_file_{}", i + 1));
    }
    mask_secrets(&mask_users(&out))
}

// ---------------------------------------------------------------- the report

/// The report. `computer`: the app's check of this computer (Settings page).
pub fn report(cat: &Catalog, id: &str, computer: &Value) -> Result<String> {
    let tools = cat.load_tools()?;
    let tool = tools.get(id).ok_or_else(|| anyhow!("unknown tool {id}"))?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Genomics Workflow Microbials: tool report\n\
         Made {}\n\n\
         This file is for the people who look after the app. It holds how the tool is set up here, what the app\n\
         said while it built, set up and tried the tool, and the script and messages of the last try. It holds no\n\
         sequence data and no results. The names of the files the tool was tried on, the computer's user name,\n\
         email addresses and anything that looks like a key, token or password are replaced (tried_file_1,\n\
         <user>, <email>, <hidden>). Please read it through before sending it.",
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
            computer["docker_error"].as_str().map(|e| format!(" ({e})")).unwrap_or_default(),
            computer["cpus"],
            computer["memory_gb"],
            computer["memory_available_gb"],
            computer["disk_free_gb"]["work"],
            computer["disk_free_gb"]["databases"],
        ),
    );

    let extra = |k: &str| {
        tool.extra
            .get(k)
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_else(|| "-".into())
    };
    section(
        &mut out,
        "The tool",
        &format!(
            "Name: {} ({})\nVersion: {}\nWhere it came from: {}\nGitHub: {}\nContainer: {}\nBuilt from: {}\nReady-made container: {}\nReady to use: {}\nNeeds databases: {}\nOther versions here: {}",
            tool.name,
            tool.id,
            tool.version,
            tool.source,
            extra("github"),
            if tool.container.is_empty() { "(none yet)" } else { &tool.container },
            extra("build"),
            extra("ready_made"),
            if tool.extra.get("draft").and_then(serde_yaml::Value::as_bool) == Some(true) { "no" } else { "yes" },
            if tool.databases.is_empty() { "-".to_string() } else { tool.databases.join(", ") },
            if tool.variants.is_empty() { "-".to_string() } else { tool.variants.keys().cloned().collect::<Vec<_>>().join(", ") },
        ),
    );
    section(
        &mut out,
        "Its definition (tool.yml)",
        &head(&read(&tool.dir.join("tool.yml")), 400),
    );

    let logs = log_dir(cat, id);
    for (kind, title) in KINDS {
        let said = read(&logs.join(format!("{kind}.txt")));
        section(
            &mut out,
            &format!("{title}: what the app said the last time"),
            &if said.is_empty() {
                "(not done, or not since the app was updated)".to_string()
            } else {
                tail(&said, 250)
            },
        );
    }

    // what the tries left: the script, what it printed, the files (names and sizes only)
    let mut names = Vec::new();
    let mut tries: Vec<PathBuf> = fs::read_dir(cat.home.join("try"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().strip_prefix(id).is_some_and(|r| r.starts_with('-')))
        })
        .collect();
    tries.sort();
    for dir in &tries {
        names.extend(private_names(dir));
        let what = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        section(
            &mut out,
            &format!("{what}: its script"),
            &head(&read(&dir.join(".command.sh")), 200),
        );
        section(
            &mut out,
            &format!("{what}: errors it printed (the end)"),
            &tail(&read(&dir.join(".command.err")), 200),
        );
        section(
            &mut out,
            &format!("{what}: other output (the end)"),
            &tail(&read(&dir.join(".command.out")), 80),
        );
        section(
            &mut out,
            &format!("{what}: files in its folder (size, name)"),
            &listing(dir),
        );
    }
    let build = cat.home.join("build").join(id);
    if build.is_dir() {
        section(&mut out, "The container build's folder (size, name)", &listing(&build));
    }

    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    names.dedup();
    let mut out = clean(&out, &names);
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
    fn secrets_and_addresses_are_left_out() {
        let said = mask_secrets(
            "export API_KEY='abc123'\nNCBI_API_KEY=xyz token: tok-en-1\n--password hunter2 password=hunter2\n\
             git clone https://jo:s3cret@github.com/x/y\nAuthorization: Bearer abcdefgh12345678\n\
             ghp_abcdefghijklmnopqrstuvwxyz0123 mail jo.smith@lab.example.org\nmlst --minid 95",
        );
        for gone in [
            "abc123",
            "xyz",
            "tok-en-1",
            "password=hunter2",
            "s3cret",
            "abcdefgh12345678",
            "ghp_abc",
            "jo.smith",
        ] {
            assert!(!said.contains(gone), "{gone} is still in:\n{said}");
        }
        // what is not a secret stays readable
        assert!(said.contains("mlst --minid 95") && said.contains("github.com/x/y"));
        assert!(said.contains("API_KEY=<hidden>") && said.contains("<email>"));
    }

    #[cfg(unix)]
    #[test]
    fn the_report_names_no_tried_file_and_no_user() {
        let home = std::env::temp_dir().join(format!("gwm-toolreport-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let dir = home.join("try").join("mytool-try");
        fs::create_dir_all(&dir).unwrap();
        fs::write(home.join("PATIENT-0042_R1.fastq.gz"), "ACGT").unwrap();
        std::os::unix::fs::symlink(
            home.join("PATIENT-0042_R1.fastq.gz"),
            dir.join("PATIENT-0042_R1.fastq.gz"),
        )
        .unwrap();
        fs::write(
            dir.join(".command.sh"),
            "export SAMPLE='PATIENT-0042' CPUS=2\nmytool /home/jo/reads/PATIENT-0042_R1.fastq.gz\n",
        )
        .unwrap();
        let names = private_names(&dir);
        assert!(names.contains(&"PATIENT-0042".to_string()), "{names:?}");
        let said = clean(&read(&dir.join(".command.sh")), &names);
        assert!(!said.contains("PATIENT") && !said.contains("/home/jo/"), "{said}");
        assert!(said.contains("tried_file_") && said.contains("/home/<user>/"));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_log_is_kept_with_its_outcome() {
        let home = std::env::temp_dir().join(format!("gwm-toollog-{}", std::process::id()));
        let cat = Catalog {
            root: home.clone(),
            home: home.clone(),
            runs: home.clone(),
        };
        keep_log(
            &cat,
            "mytool",
            "setup",
            &["Reading the help".into()],
            Some(&anyhow!("no help text")),
        );
        let said = read(&log_dir(&cat, "mytool").join("setup.txt"));
        assert!(said.contains("FAILED") && said.contains("Reading the help") && said.contains("Error: no help text"));
        forget(&cat, "mytool");
        assert!(!log_dir(&cat, "mytool").exists());
        let _ = fs::remove_dir_all(&home);
    }
}
