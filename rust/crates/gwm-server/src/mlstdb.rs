//! MLST typing's up-to-date database. The tool comes with PubMLST data
//! submitted up to 31 December 2024 (inside its container). PubMLST gives
//! newer data only to a user's own account, so the user can enter their own
//! key here: the tool (mlst-typing 0.5.0 and newer, in its container) tests
//! it, keeps it on this computer and downloads the current schemes into a
//! database of the user's own. The MLST typing tool page and the Databases
//! page both show this.
//!
//! Folders, under the databases folder:
//!   mlst_typing/own/   what a typing step is given as `$USER_DATA` (the
//!                      tool's `user_data`): `in_use` (builtin | own) and
//!                      the tool's data folder (`mlst-typing/database`)
//!   mlst_typing/key/   the tool's config folder: the saved key. Never
//!                      given to a typing step.
//!   mlst_typing/job/   the folder the setup command runs in
//!
//! The key goes to the tool through a file that is deleted as soon as the
//! tool has read it; it is never put on a command line or in a log.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use gwm_core::{compiler, paths, Catalog};
use gwm_engine::backend::{self, Backend, TaskRun};
use serde_json::{json, Value};

use crate::jobs::{self, Job, Reporter};

/// The built-in tool this belongs to.
pub const TOOL: &str = "mlst_typing";
/// One setup command at a time, in a container of this name.
const CONTAINER: &str = "gwm-mlst-database";
/// The tool inside its container.
const PROGRAM: &str = "python /opt/mlst_typing/mlst_typing.py";

/// The job in memory: (what it does, its id in `jobs`).
static CURRENT: Mutex<Option<(String, String)>> = Mutex::new(None);

fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[derive(Debug, Clone)]
struct Dirs {
    base: PathBuf,
    own: PathBuf,
    key: PathBuf,
    job: PathBuf,
    /// the tool's container
    image: String,
}

impl Dirs {
    /// Nothing when the MLST typing tool here has no own database (an older one).
    fn of(cat: &Catalog) -> Result<Option<Dirs>> {
        let tools = cat.load_tools()?;
        let Some(tool) = tools.get(TOOL) else { return Ok(None) };
        let Some(rel) = compiler::user_data(tool) else {
            return Ok(None);
        };
        let own = paths::db_dir(&cat.home).join(rel);
        let base = own.parent().unwrap_or(&own).to_path_buf();
        Ok(Some(Dirs {
            key: base.join("key"),
            job: base.join("job"),
            base,
            own,
            image: tool.container.clone(),
        }))
    }

    fn need(cat: &Catalog) -> Result<Dirs> {
        Dirs::of(cat)?.ok_or_else(|| anyhow!("this version of MLST typing has no database of its own to download"))
    }

    fn database(&self) -> PathBuf {
        self.own.join("mlst-typing").join("database")
    }

    fn has_own(&self) -> bool {
        self.database().join("blast").join("mlst.fa.nsq").exists()
    }

    fn in_use_file(&self) -> PathBuf {
        self.own.join("in_use")
    }

    /// What the user chose (builtin | own); nothing until they choose.
    fn chosen(&self) -> Option<String> {
        let text = fs::read_to_string(self.in_use_file()).ok()?;
        let text = text.trim();
        matches!(text, "builtin" | "own").then(|| text.to_string())
    }

    fn version(&self) -> String {
        fs::read_to_string(self.database().join("VERSION.txt"))
            .ok()
            .and_then(|t| t.lines().next().map(|l| l.trim().to_string()))
            .unwrap_or_default()
    }

    fn tool_file(&self, name: &str) -> PathBuf {
        self.key.join("mlst-typing").join(name)
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// PubMLST schemes of the own database that still hold only the free data.
fn limited_schemes(d: &Dirs) -> usize {
    let Ok(text) = fs::read_to_string(d.database().join("manifest.tsv")) else {
        return 0;
    };
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
    let col = |name: &str| header.iter().position(|h| *h == name);
    let (Some(source), Some(restricted)) = (col("source"), col("restricted")) else {
        return 0;
    };
    lines
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .filter(|f| f.get(source) == Some(&"pubmlst") && f.get(restricted) == Some(&"True"))
        .count()
}

/// `[12/162] ecoli_achtman_4` -> (12, 162, "ecoli_achtman_4")
fn scheme_line(line: &str) -> Option<(u64, u64, &str)> {
    let rest = line.strip_prefix('[')?;
    let (count, name) = rest.split_once("] ")?;
    let (i, n) = count.split_once('/')?;
    Some((i.parse().ok()?, n.parse().ok()?, name.trim()))
}

/// The line that says what went wrong, from what the tool printed.
fn problem(err: &str, exit: i32) -> String {
    let lines: Vec<&str> = err.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("ERROR: "))
        .or(lines.last().copied())
        .map(str::to_string)
        .unwrap_or_else(|| format!("the tool stopped with exit code {exit}"))
}

/// Follow what the tool prints while it runs: the scheme count is the
/// progress, every other line is the latest news.
fn watch(file: &Path, r: &Reporter, done: &AtomicBool) {
    let mut seen = 0;
    loop {
        let finished = done.load(Ordering::Relaxed);
        if let Ok(text) = fs::read_to_string(file) {
            let lines: Vec<&str> = text.lines().collect();
            // the last line may still be half written
            let whole = if finished || text.ends_with('\n') {
                lines.len()
            } else {
                lines.len().saturating_sub(1)
            };
            for line in lines.iter().take(whole).skip(seen) {
                let line = line.trim();
                if let Some((i, n, name)) = scheme_line(line) {
                    // the count is of schemes started: the bar fills when the last one is done
                    r.progress(i.saturating_sub(1), n);
                    r.progress_line(&format!("Scheme {i} of {n}: {name}"));
                } else if !line.is_empty() && !line.starts_with("FAILED") {
                    r.progress_line(line);
                }
            }
            seen = seen.max(whole);
        }
        if finished {
            return;
        }
        std::thread::sleep(Duration::from_millis(700));
    }
}

#[cfg(unix)]
fn private(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn private(_path: &Path) {}

/// Run one setup command of the tool in its container and return what it
/// printed. `secret` (a key) is handed over in a file the command deletes.
fn run(
    d: &Dirs,
    backend: &dyn Backend,
    kind: &str,
    body: &str,
    secret: Option<&str>,
    r: Option<&Reporter>,
) -> Result<String> {
    for dir in [&d.own, &d.key, &d.job] {
        fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    for f in [".command.out", ".command.err", "secret"] {
        let _ = fs::remove_file(d.job.join(f));
    }
    let secret_file = d.job.join("secret");
    if let Some(s) = secret {
        fs::write(&secret_file, "")?;
        private(&secret_file);
        fs::write(&secret_file, format!("{s}\n"))?;
    }
    // `running` says a command is going, also to an app that was closed and opened again
    let script = format!(
        "#!/bin/bash -ue\nexport XDG_CONFIG_HOME={} XDG_DATA_HOME={} HOME=/tmp\necho {kind} > running\ntrap 'rm -f running secret' EXIT\n{body}\n",
        sq(&d.key.to_string_lossy()),
        sq(&d.own.to_string_lossy()),
    );
    fs::write(d.job.join(".command.sh"), script)?;
    let task = TaskRun {
        name: CONTAINER.into(),
        dir: d.job.clone(),
        image: d.image.clone(),
        cpus: 1,
        memory_gb: 4,
        mounts: vec![(d.base.clone(), false)],
    };
    let done = AtomicBool::new(false);
    let err_file = d.job.join(".command.err");
    let exit = std::thread::scope(|s| {
        if let Some(r) = r {
            s.spawn(|| watch(&err_file, r, &done));
        }
        let exit = backend.run(&task, &AtomicBool::new(false));
        done.store(true, Ordering::Relaxed);
        exit
    });
    let _ = fs::remove_file(&secret_file);
    let _ = fs::remove_file(d.job.join("running"));
    let exit = exit?;
    if exit != 0 {
        bail!("{}", problem(&fs::read_to_string(&err_file).unwrap_or_default(), exit));
    }
    Ok(fs::read_to_string(d.job.join(".command.out")).unwrap_or_default())
}

/// After a download: the own database, when there is one now, is the one in use.
fn use_own(d: &Dirs) -> String {
    format!(
        "if [ -f {}/blast/mlst.fa.nsq ]; then echo own > {}; fi",
        sq(&d.database().to_string_lossy()),
        sq(&d.in_use_file().to_string_lossy())
    )
}

/// A command still going in a container that this engine did not start (the
/// app was closed and opened again during a download).
fn left_running(d: &Dirs) -> Option<Value> {
    let marker = d.job.join("running");
    let kind = fs::read_to_string(&marker).ok()?.trim().to_string();
    let alive = gwm_engine::docker::command()
        .args(["ps", "-q", "--filter", &format!("name=^{CONTAINER}$")])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.iter().all(u8::is_ascii_whitespace));
    if !alive {
        let _ = fs::remove_file(&marker);
        return None;
    }
    let text = fs::read_to_string(d.job.join(".command.err")).unwrap_or_default();
    let last = text.lines().rev().find_map(scheme_line);
    Some(json!({
        "kind": kind,
        "status": "running",
        "progress": last.map(|(i, n, _)| [i.saturating_sub(1), n]),
        "message": last.map(|(i, n, name)| format!("Scheme {i} of {n}: {name}")),
    }))
}

fn job_view(kind: &str, job: &Job) -> Value {
    json!({
        "kind": kind,
        "id": job.id,
        "status": job.status,
        "progress": job.progress,
        "message": job.log.last().map(|l| l.strip_prefix("Error: ").unwrap_or(l)),
        "result": job.result,
    })
}

fn current(d: &Dirs) -> Option<Value> {
    let held = CURRENT.lock().unwrap().clone();
    if let Some((kind, id)) = held {
        if let Some(job) = jobs::get(&id) {
            return Some(job_view(&kind, &job));
        }
    }
    left_running(d)
}

fn busy(d: &Dirs) -> bool {
    current(d).is_some_and(|j| j["status"] == "running")
}

/// Where things stand, for the panel and the Home page. Never holds the key.
pub fn status(cat: &Catalog) -> Result<Value> {
    let Some(d) = Dirs::of(cat)? else {
        return Ok(json!({"available": false}));
    };
    let cred = read_json(&d.tool_file("credentials.json"));
    let own = d.has_own();
    let chosen = d.chosen();
    Ok(json!({
        "available": true,
        // false until the user picks the pre-2025 database or enters a key
        "answered": chosen.is_some() || cred.is_some(),
        "in_use": if own && chosen.as_deref() == Some("own") { "own" } else { "builtin" },
        "key_saved": cred.is_some(),
        "key_type": cred.as_ref().and_then(|c| c["type"].as_str()).unwrap_or(""),
        "oauth_waiting": d.tool_file("oauth_pending.json").exists(),
        "own_database": own,
        "own_version": d.version(),
        "own_limited_schemes": limited_schemes(&d),
        "last_check": read_json(&d.base.join("last_check.json")),
        "job": current(&d),
    }))
}

/// Is a download (or a check) going? The desktop app keeps the services on for it.
pub fn working(cat: &Catalog) -> bool {
    Dirs::of(cat).ok().flatten().is_some_and(|d| busy(&d))
}

/// Choose the database typing uses from now on: `builtin` (pre-2025) or `own`.
pub fn choose(cat: &Catalog, which: &str) -> Result<()> {
    let d = Dirs::need(cat)?;
    match which {
        "builtin" => {}
        "own" if d.has_own() => {}
        "own" => bail!("your up-to-date database is not downloaded yet: enter your PubMLST key first"),
        _ => bail!("unknown database {which}"),
    }
    fs::create_dir_all(&d.own)?;
    fs::write(d.in_use_file(), format!("{which}\n"))?;
    Ok(())
}

fn containers() -> Result<Box<dyn Backend>> {
    let backend = backend::detect();
    if backend.name() == "docker" && gwm_engine::docker::probe().version.is_none() {
        bail!("the tools can't start yet (Docker is not running): see Settings");
    }
    Ok(backend)
}

/// Start a setup command in the background; the panel follows it.
fn start(
    cat: &Catalog,
    kind: &'static str,
    title: &str,
    body: String,
    secret: Option<String>,
    after: impl FnOnce(&Dirs, &str, &Reporter) -> Result<()> + Send + 'static,
) -> Result<Value> {
    let d = Dirs::need(cat)?;
    let mut held = CURRENT.lock().unwrap();
    let going = held
        .as_ref()
        .and_then(|(_, id)| jobs::get(id))
        .is_some_and(|j| j.status == "running");
    if going || left_running(&d).is_some() {
        bail!("the MLST database is already being downloaded or checked: wait for it to finish");
    }
    // say so now when the tools can't start, not a moment later as a failed download
    containers()?;
    let work = d.clone();
    let job = jobs::spawn(title, move |r| {
        let out = run(&work, containers()?.as_ref(), kind, &body, secret.as_deref(), Some(r))?;
        after(&work, &out, r)
    });
    *held = Some((kind.to_string(), job.id.clone()));
    Ok(job_view(kind, &job))
}

/// What a download did, for the panel's message.
fn downloaded(before: (bool, String)) -> impl FnOnce(&Dirs, &str, &Reporter) -> Result<()> {
    move |d, _, r| {
        let own = d.has_own();
        r.progress(1, 1);
        r.result(json!({
            "own_database": own,
            "changed": own && (!before.0 || before.1 != d.version()),
        }));
        // an earlier check's answer no longer holds
        let _ = fs::remove_file(d.base.join("last_check.json"));
        Ok(())
    }
}

fn one_word(text: &str, what: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() || text.split_whitespace().count() != 1 || text.len() > 400 {
        bail!("{what} is one word without spaces: paste it exactly as PubMLST shows it");
    }
    Ok(text.to_string())
}

/// The user's PubMLST "Public data access key": test it, keep it, download.
pub fn add_key(cat: &Catalog, key: &str) -> Result<Value> {
    let key = one_word(key, "the key")?;
    let d = Dirs::need(cat)?;
    let body = format!(
        "key=$(cat secret); rm -f secret\nprintf '%s\\n' \"$key\" | {PROGRAM} --setup --key-file -\n{}",
        use_own(&d)
    );
    let before = (d.has_own(), d.version());
    start(
        cat,
        "download",
        "Download the up-to-date MLST database",
        body,
        Some(key),
        downloaded(before),
    )
}

/// OAuth, step 1: the client key and secret. Answers with the PubMLST page
/// where the user approves the tool and gets a verification code.
pub fn oauth_start(cat: &Catalog, client_key: &str, client_secret: &str) -> Result<Value> {
    let (client_key, client_secret) = (
        one_word(client_key, "the client key")?,
        one_word(client_secret, "the client secret")?,
    );
    let d = Dirs::need(cat)?;
    if busy(&d) {
        bail!("the MLST database is already being downloaded or checked: wait for it to finish");
    }
    let out = run(
        &d,
        containers()?.as_ref(),
        "sign-in",
        &format!("{PROGRAM} --setup --oauth-start --key-file secret"),
        Some(&format!("{client_key}\n{client_secret}")),
        None,
    )?;
    let page = serde_json::from_str::<Value>(&out)
        .ok()
        .and_then(|v| v["approve_url"].as_str().map(str::to_string))
        .context("the tool did not give the PubMLST page to approve it on")?;
    Ok(json!({"approve_url": page}))
}

/// OAuth, step 2: the verification code PubMLST showed. Keeps the key and downloads.
pub fn oauth_finish(cat: &Catalog, code: &str) -> Result<Value> {
    let code = one_word(code, "the verification code")?;
    if !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("the verification code has only letters and numbers: copy it from the PubMLST page");
    }
    let d = Dirs::need(cat)?;
    if !d.tool_file("oauth_pending.json").exists() {
        bail!("enter the client key and client secret first");
    }
    let body = format!("{PROGRAM} --setup --oauth-finish {}\n{}", sq(&code), use_own(&d));
    let before = (d.has_own(), d.version());
    start(
        cat,
        "download",
        "Download the up-to-date MLST database",
        body,
        None,
        downloaded(before),
    )
}

/// Download again with the saved key. The choice of database stays, unless
/// this is the first download that worked.
pub fn update(cat: &Catalog) -> Result<Value> {
    let d = Dirs::need(cat)?;
    if !d.tool_file("credentials.json").exists() {
        bail!("no PubMLST key is saved yet: enter your key first");
    }
    let first = if d.has_own() { String::new() } else { use_own(&d) };
    let body = format!("{PROGRAM} --update\n{first}");
    let before = (d.has_own(), d.version());
    start(
        cat,
        "download",
        "Update the MLST database",
        body,
        None,
        downloaded(before),
    )
}

/// Ask PubMLST whether anything is newer than the database here. Downloads nothing.
pub fn check(cat: &Catalog) -> Result<Value> {
    let d = Dirs::need(cat)?;
    if !d.tool_file("credentials.json").exists() {
        bail!("no PubMLST key is saved yet: enter your key first");
    }
    start(
        cat,
        "check",
        "Check for a newer MLST database",
        format!("{PROGRAM} --update-check"),
        None,
        |d, out, r| {
            let found: Value = serde_json::from_str(out).context("reading what the check found")?;
            let count = |k: &str| found[k].as_array().map_or(0, Vec::len);
            let result = json!({
                "newer": found["newer"].as_bool().unwrap_or(false),
                "schemes_newer": count("schemes_newer"),
                "schemes_checked": found["schemes_checked"],
                "schemes_failed": count("schemes_failed"),
                "checked": chrono::Local::now().to_rfc3339(),
            });
            fs::write(d.base.join("last_check.json"), serde_json::to_string(&result)?)?;
            r.progress(1, 1);
            r.result(result);
            Ok(())
        },
    )
}

/// Forget the key and delete the downloaded database: back to the pre-2025 one.
pub fn remove(cat: &Catalog) -> Result<()> {
    let d = Dirs::need(cat)?;
    if busy(&d) {
        bail!("the MLST database is being downloaded or checked: wait for it to finish");
    }
    for dir in [d.key.clone(), d.own.join("mlst-typing"), d.job.clone()] {
        if dir.exists() {
            fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
    }
    let _ = fs::remove_file(d.base.join("last_check.json"));
    *CURRENT.lock().unwrap() = None;
    fs::create_dir_all(&d.own)?;
    fs::write(d.in_use_file(), "builtin\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of this test's own, emptied first.
    fn dirs(test: &str) -> Dirs {
        let base = std::env::temp_dir()
            .join(format!("gwm-mlstdb-{test}-{}", std::process::id()))
            .join("mlst_typing");
        let _ = fs::remove_dir_all(&base);
        Dirs {
            own: base.join("own"),
            key: base.join("key"),
            job: base.join("job"),
            base,
            image: String::new(),
        }
    }

    #[test]
    fn reads_what_the_tool_prints() {
        assert_eq!(
            scheme_line("[12/162] ecoli_achtman_4"),
            Some((12, 162, "ecoli_achtman_4"))
        );
        assert_eq!(scheme_line("Testing the key with PubMLST ..."), None);
        assert_eq!(
            problem(
                "Testing the key with PubMLST ...\nERROR: PubMLST answered HTTP 401: bad key\n",
                1
            ),
            "PubMLST answered HTTP 401: bad key"
        );
        assert_eq!(problem("", 2), "the tool stopped with exit code 2");
        assert!(one_word("abc def", "the key").is_err());
        assert_eq!(one_word(" abc\n", "the key").unwrap(), "abc");
    }

    /// The whole route with bash in place of the container: the key reaches
    /// the tool through the file and is gone afterwards, the scheme count is
    /// the progress, a failure gives the tool's own words.
    #[cfg(unix)]
    #[test]
    fn runs_a_setup_command() {
        let d = dirs("run");
        let job = jobs::spawn("test", {
            let d = d.clone();
            move |r| {
                let body = format!(
                    "key=$(cat secret); rm -f secret\necho \"[1/2] a\" >&2; echo \"[2/2] b\" >&2\n\
                     mkdir -p {db}/blast; touch {db}/blast/mlst.fa.nsq; echo \"got $key\"\n{}",
                    use_own(&d),
                    db = sq(&d.database().to_string_lossy())
                );
                let out = run(&d, &backend::Local, "download", &body, Some("k3y"), Some(r))?;
                assert_eq!(out.trim(), "got k3y");
                Ok(())
            }
        });
        let done = loop {
            let j = jobs::get(&job.id).unwrap();
            if j.status != "running" {
                break j;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(done.status, "done", "{:?}", done.log);
        assert_eq!(done.progress, Some((1, 2)));
        assert!(done.log.iter().any(|l| l == "Scheme 2 of 2: b"));
        assert!(!d.job.join("secret").exists() && !d.job.join("running").exists());
        assert!(d.has_own());
        assert_eq!(d.chosen().as_deref(), Some("own"));
        // the key is in no file the command left behind
        for f in [".command.sh", ".command.out", ".command.err"] {
            let text = fs::read_to_string(d.job.join(f)).unwrap();
            assert!(f == ".command.out" || !text.contains("k3y"), "{f}");
        }

        let e = run(
            &d,
            &backend::Local,
            "download",
            "echo 'ERROR: PubMLST answered HTTP 401' >&2; exit 1",
            Some("k"),
            None,
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "PubMLST answered HTTP 401");
        assert!(!d.job.join("secret").exists());
        let _ = fs::remove_dir_all(d.base.parent().unwrap());
    }

    #[test]
    fn counts_schemes_left_on_the_free_data() {
        let d = dirs("count");
        fs::create_dir_all(d.database()).unwrap();
        fs::write(
            d.database().join("manifest.tsv"),
            "scheme\tsource\tloci\talleles\tprofiles\tlast_updated\trestricted\ttaxon\tapi\n\
             a\tpubmlst\t7\t1\t1\t2026-01-01\tFalse\t\t\n\
             b\tpubmlst\t7\t1\t1\t2024-01-01\tTrue\t\t\n\
             c\tenterobase\t7\t1\t1\t2026-01-01\tFalse\t\t\n",
        )
        .unwrap();
        assert_eq!(limited_schemes(&d), 1);
        assert_eq!(d.chosen(), None);
        fs::write(d.in_use_file(), "own\n").unwrap();
        assert_eq!(d.chosen().as_deref(), Some("own"));
        let _ = fs::remove_dir_all(d.base.parent().unwrap());
    }
}
