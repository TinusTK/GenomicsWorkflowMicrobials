//! Kun-peng's copies of the Kraken2 databases. Kun-peng does not read a
//! Kraken2 database as it is: the one big hash table is cut into pieces
//! first, so that only one piece at a time is held in memory. The app makes
//! that copy from a Kraken2 database the user installed, with the tool's own
//! `prepare.sh` in the tool's container: by itself the first time a Kun-peng
//! step needs it, or ahead of time from the Databases page (here).
//!
//! The copy stands on its own, so afterwards the Kraken2 database can be
//! removed (Kun-peng keeps working) or the copy can (Kraken2 keeps working).
//!
//! Folders, under the databases folder:
//!   kun_peng/<database id>/   the copy (the tool's `user_data`, `$USER_DATA`)
//!   kun_peng/locks/           one copy is made at a time (prepare.sh)
//!   kun_peng/job-<id>/        the folder the Databases page's job runs in

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use gwm_core::catalog::{Database, Step, Tool};
use gwm_core::{compiler, paths, value, Catalog};
use gwm_engine::backend::TaskRun;
use indexmap::IndexMap;
use serde_json::{json, Value};

use crate::jobs::{self, Job, Reporter};
use crate::{cleanup, mlstdb};

/// The built-in tool this belongs to.
pub const TOOL: &str = "kun_peng";
/// The tool's script that makes a copy.
const SCRIPT: &str = "prepare.sh";
/// The file that says a copy is whole (written last).
const READY: &str = "hash_config.k2d";

/// database id -> the job making its copy
static JOBS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The Kun-peng tool here, when it keeps copies of databases.
struct Setup {
    /// `<databases folder>/kun_peng`
    root: PathBuf,
    image: String,
    script: PathBuf,
    /// the Kraken2 databases it can use
    ids: Vec<String>,
}

fn setup(cat: &Catalog) -> Result<Option<Setup>> {
    let tools = cat.load_tools()?;
    Ok(tools.get(TOOL).and_then(of_tool).map(|(rel, ids)| Setup {
        root: paths::db_dir(&cat.home).join(rel),
        image: tools[TOOL].container.clone(),
        script: tools[TOOL].dir.join(SCRIPT),
        ids,
    }))
}

/// (the folder of its copies, the databases it can use) of a tool that
/// makes its own copies of databases.
fn of_tool(tool: &Tool) -> Option<(String, Vec<String>)> {
    let ids = compiler::optional_databases(tool);
    let rel = compiler::user_data(tool)?;
    (compiler::user_data_writable(tool) && !ids.is_empty()).then_some((rel, ids))
}

fn need(cat: &Catalog, id: &str) -> Result<Setup> {
    let s = setup(cat)?.ok_or_else(|| anyhow!("this version of the app has no Kun-peng tool"))?;
    if !s.ids.iter().any(|d| d == id) {
        bail!("unknown database {id} for Kun-peng");
    }
    Ok(s)
}

fn has_copy(root: &Path, id: &str) -> bool {
    root.join(id).join(READY).is_file()
}

fn job_of(id: &str) -> Option<Job> {
    let held = JOBS.lock().unwrap();
    held.as_ref()?.get(id).and_then(|j| jobs::get(j))
}

fn preparing(id: &str) -> bool {
    job_of(id).is_some_and(|j| j.status == "running")
}

/// Is a copy being made from the Databases page? The desktop app keeps the
/// services on for it.
pub fn working() -> bool {
    let held = JOBS.lock().unwrap();
    held.iter()
        .flat_map(|m| m.values())
        .any(|j| jobs::get(j).is_some_and(|j| j.status == "running"))
}

/// The databases as the Databases page shows them: those Kun-peng can use
/// also say whether their copy is there.
pub fn list(cat: &Catalog) -> Result<Vec<Value>> {
    let s = setup(cat)?;
    let mut out = Vec::new();
    for (id, db) in cat.load_databases()? {
        let mut v = serde_json::to_value(&db)?;
        if let Some(s) = s.as_ref().filter(|s| s.ids.contains(&id)) {
            let dir = s.root.join(&id);
            let prepared = has_copy(&s.root, &id);
            v["kun_peng"] = json!({
                "prepared": prepared,
                "path": dir,
                "bytes": if prepared { cleanup::folder_bytes(&dir) } else { 0 },
                // the Kraken2 database it was made from (its folder name carries the date)
                "source": fs::read_to_string(dir.join("source.txt")).map(|t| t.trim().to_string()).unwrap_or_default(),
                "preparing": preparing(&id),
            });
        }
        out.push(v);
    }
    Ok(out)
}

/// Follow the copy as it grows: its size against the Kraken2 hash table's.
fn watch(part: &Path, total: u64, r: &Reporter, done: &AtomicBool) {
    while !done.load(Ordering::Relaxed) {
        if total > 0 {
            r.progress(cleanup::folder_bytes(part).min(total), total);
        }
        std::thread::sleep(Duration::from_millis(700));
    }
}

/// Make the copy of one Kraken2 database (the job's work).
fn make(s: &Setup, db: &Database, r: &Reporter) -> Result<()> {
    let dest = s.root.join(&db.id);
    let job = s.root.join(format!("job-{}", db.id));
    let _ = fs::remove_dir_all(&job);
    fs::create_dir_all(&job).with_context(|| format!("making {}", job.display()))?;
    fs::copy(&s.script, job.join(SCRIPT)).with_context(|| format!("reading {}", s.script.display()))?;
    fs::write(
        job.join(".command.sh"),
        format!(
            "#!/bin/bash -ue\nbash {SCRIPT} {} {}\n",
            sq(&db.path.to_string_lossy()),
            sq(&dest.to_string_lossy())
        ),
    )?;
    let task = TaskRun {
        name: format!("gwm-kun-peng-{}", db.id.replace('_', "-")),
        dir: job.clone(),
        image: s.image.clone(),
        cpus: 1,
        memory_gb: 4,
        mounts: vec![(s.root.clone(), false), (db.path.clone(), true)],
    };
    r.log(format!("Cutting {} into pieces for Kun-peng", db.name));
    let total = fs::metadata(db.path.join("hash.k2d")).map(|m| m.len()).unwrap_or(0);
    let part = s.root.join(format!("{}.partial", db.id));
    let backend = mlstdb::containers()?;
    let done = AtomicBool::new(false);
    let exit = std::thread::scope(|scope| {
        scope.spawn(|| watch(&part, total, r, &done));
        let exit = backend.run(&task, &AtomicBool::new(false));
        done.store(true, Ordering::Relaxed);
        exit
    })?;
    let err = fs::read_to_string(job.join(".command.err")).unwrap_or_default();
    let _ = fs::remove_dir_all(&job);
    if exit != 0 {
        bail!("{}", mlstdb::problem(&err, exit));
    }
    if !has_copy(&s.root, &db.id) {
        bail!("the copy was not made");
    }
    r.progress(1, 1);
    r.log(format!("{} is ready for Kun-peng", db.name));
    Ok(())
}

/// Databases page, "Prepare for Kun-peng": make the copy in the background.
pub fn prepare(cat: &Catalog, id: &str) -> Result<Job> {
    let s = need(cat, id)?;
    let dbs = cat.load_databases()?;
    let db = dbs.get(id).cloned().with_context(|| format!("unknown database {id}"))?;
    if has_copy(&s.root, id) {
        bail!("{} is already prepared for Kun-peng", db.name);
    }
    if !db.installed {
        bail!(
            "{} is not installed: install it first, the Kun-peng copy is made from it",
            db.name
        );
    }
    let mut held = JOBS.lock().unwrap();
    let map = held.get_or_insert_with(HashMap::new);
    if map
        .get(id)
        .and_then(|j| jobs::get(j))
        .is_some_and(|j| j.status == "running")
    {
        bail!("{} is already being prepared for Kun-peng", db.name);
    }
    // say so now when the tools can't start, not a moment later as a failed job
    mlstdb::containers()?;
    let job = jobs::spawn(&format!("Prepare {} for Kun-peng", db.name), move |r| make(&s, &db, r));
    map.insert(id.to_string(), job.id.clone());
    Ok(job)
}

fn no_runs(cat: &Catalog) -> Result<()> {
    let going = cleanup::active_runs(cat);
    if going > 0 {
        bail!("{going} run(s) are still going. Wait for them to finish, or stop them, then try again.");
    }
    Ok(())
}

/// Remove the Kun-peng copy of a database. The Kraken2 database itself stays.
pub fn remove_copy(cat: &Catalog, id: &str) -> Result<()> {
    let s = need(cat, id)?;
    if preparing(id) {
        bail!("the Kun-peng copy is being made right now: wait for it to finish");
    }
    no_runs(cat)?;
    for dir in [s.root.join(id), s.root.join(format!("{id}.partial"))] {
        if dir.exists() {
            fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
    }
    Ok(())
}

/// Remove an installed database from this computer (it can be installed
/// again). A Kun-peng copy made from it stays and keeps working.
pub fn remove_database(cat: &Catalog, id: &str) -> Result<()> {
    let dbs = cat.load_databases()?;
    let db = dbs.get(id).with_context(|| format!("unknown database {id}"))?;
    if preparing(id) {
        bail!("its Kun-peng copy is being made right now: wait for it to finish");
    }
    no_runs(cat)?;
    let root = paths::db_dir(&cat.home);
    // only ever a folder of its own inside the databases folder
    if db.install_dir.is_empty()
        || db.install_dir.split('/').any(|p| p.is_empty() || p == "..")
        || !db.path.starts_with(&root)
    {
        bail!("{} can't be removed from here", db.name);
    }
    if db.path.is_dir() {
        fs::remove_dir_all(&db.path).with_context(|| format!("removing {}", db.path.display()))?;
    }
    Ok(())
}

/// For the check before a run: what a step of a tool that makes its own
/// copies of databases (Kun-peng) will find. `Err` = it can't run here;
/// `Ok(Some(..))` = a note (the copy is made first).
pub fn step_check(
    cat: &Catalog,
    tool: &Tool,
    step: &Step,
    dbs: &IndexMap<String, Database>,
) -> Result<Option<String>, String> {
    let Some((rel, ids)) = of_tool(tool) else {
        return Ok(None);
    };
    let root = paths::db_dir(&cat.home).join(rel);
    let installed = |id: &str| dbs.get(id).is_some_and(|d| d.installed);
    let name = |id: &str| dbs.get(id).map_or(id.to_string(), |d| d.name.clone());
    let picked = step
        .settings
        .get("database")
        .or_else(|| tool.settings.get("database").map(|s| &s.default))
        .map(value::to_text)
        .unwrap_or_default();
    let first_made = |id: &str| {
        let gb = dbs.get(id).map_or(0.0, |d| d.size_gb);
        format!(
            "the first sample prepares the {} database for {} first (once; it takes about {gb} GB of disk and some minutes).",
            name(id),
            tool.name
        )
    };
    if picked.is_empty() || picked == "auto" {
        if ids.iter().any(|id| has_copy(&root, id)) {
            return Ok(None);
        }
        // the biggest installed one: the list in the tool.yml is smallest first
        return match ids.iter().rev().find(|id| installed(id)) {
            Some(id) => Ok(Some(first_made(id))),
            None => Err(format!(
                "{} needs a Kraken2 database, and none is installed. Install one on the Databases page.",
                tool.name
            )),
        };
    }
    if has_copy(&root, &picked) {
        Ok(None)
    } else if installed(&picked) {
        Ok(Some(first_made(&picked)))
    } else {
        Err(format!(
            "{} needs the {} database, which is not installed and has no {} copy. Install it on the Databases page, or pick another one in the step's settings.",
            tool.name,
            name(&picked),
            tool.name
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwm_engine::backend::{self, Backend};

    fn tool() -> Tool {
        serde_yaml::from_str(
            "id: kun_peng\nname: Kun-peng\nuser_data: kun_peng\nuser_data_writable: true\n\
             optional_databases: [k8, k16]\n\
             settings:\n  database: {type: select, default: auto}\n",
        )
        .unwrap()
    }

    fn database(id: &str, installed: bool) -> Database {
        let mut db: Database =
            serde_yaml::from_str(&format!("name: Kraken2 {id}\ninstall_dir: kraken2/{id}\nsize_gb: 8\n")).unwrap();
        db.id = id.into();
        db.installed = installed;
        db
    }

    /// A data folder of this test's own, emptied first.
    fn home(test: &str) -> Catalog {
        let home = std::env::temp_dir().join(format!("gwm-kunpeng-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        Catalog::new(home.join("root"), home)
    }

    #[test]
    fn says_what_a_step_will_find() {
        let cat = home("check");
        let root = paths::db_dir(&cat.home).join("kun_peng");
        let tool = tool();
        let auto = Step::default();
        let mut k16 = Step::default();
        k16.settings.insert("database".into(), "k16".into());
        let dbs = |a: bool, b: bool| {
            IndexMap::from([
                ("k8".to_string(), database("k8", a)),
                ("k16".to_string(), database("k16", b)),
            ])
        };

        // nothing here: it can't run, and the message sends the user to the Databases page
        let e = step_check(&cat, &tool, &auto, &dbs(false, false)).unwrap_err();
        assert!(e.contains("Databases page") && e.contains("database"), "{e}");
        // a Kraken2 database but no copy yet: it runs, the copy is made first (the biggest one)
        let note = step_check(&cat, &tool, &auto, &dbs(true, true)).unwrap().unwrap();
        assert!(note.contains("Kraken2 k16"), "{note}");
        // a chosen database that is neither installed nor copied
        assert!(step_check(&cat, &tool, &k16, &dbs(true, false)).is_err());

        // the copy is there and the Kraken2 database was removed: it runs, nothing to say
        fs::create_dir_all(root.join("k16")).unwrap();
        fs::write(root.join("k16").join(READY), "").unwrap();
        assert_eq!(step_check(&cat, &tool, &k16, &dbs(true, false)), Ok(None));
        assert_eq!(step_check(&cat, &tool, &auto, &dbs(false, false)), Ok(None));
        // a tool without copies of its own is not looked at
        let plain: Tool = serde_yaml::from_str("id: x\nname: X\n").unwrap();
        assert_eq!(step_check(&cat, &plain, &auto, &dbs(false, false)), Ok(None));
        let _ = fs::remove_dir_all(&cat.home);
    }

    /// The real prepare.sh with a stand-in for the kun_peng program: the copy
    /// is made beside the others, stands on its own (no link to the Kraken2
    /// database is left in it) and a second go finds the work done.
    #[cfg(target_os = "linux")]
    #[test]
    fn makes_a_copy_that_stands_on_its_own() {
        let cat = home("copy");
        let dir = &cat.home;
        let src = dir.join("kraken2").join("k2_test_20260101");
        fs::create_dir_all(&src).unwrap();
        for f in ["hash.k2d", "opts.k2d", "taxo.k2d"] {
            fs::write(src.join(f), f).unwrap();
        }
        // stands in for `kun_peng hashshard --db <dir> ...`: writes the pieces beside hash.k2d
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(
            bin.join("kun_peng"),
            "#!/bin/bash\n[ \"$1\" = hashshard ] || exit 2\ncat \"$3/hash.k2d\" > \"$3/hash_1.k2d\"\necho cfg > \"$3/hash_config.k2d\"\n",
        )
        .unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tools/kun_peng")
            .join(SCRIPT);
        let root = dir.join("kun_peng");
        let job = root.join("job");
        fs::create_dir_all(&job).unwrap();
        let dest = root.join("k8");
        let run = |src: &Path| {
            fs::write(
                job.join(".command.sh"),
                format!(
                    "chmod +x {bin}/kun_peng\nexport PATH={bin}:$PATH\nbash {} {} {}\n",
                    sq(&script.to_string_lossy()),
                    sq(&src.to_string_lossy()),
                    sq(&dest.to_string_lossy()),
                    bin = sq(&bin.to_string_lossy()),
                ),
            )
            .unwrap();
            let task = TaskRun {
                name: "t".into(),
                dir: job.clone(),
                image: String::new(),
                cpus: 1,
                memory_gb: 1,
                mounts: vec![],
            };
            backend::Local.run(&task, &AtomicBool::new(false)).unwrap()
        };
        assert_eq!(
            run(&src),
            0,
            "{}",
            fs::read_to_string(job.join(".command.err")).unwrap()
        );
        assert!(has_copy(&root, "k8"));
        assert_eq!(fs::read_to_string(dest.join("hash_1.k2d")).unwrap(), "hash.k2d");
        assert_eq!(
            fs::read_to_string(dest.join("source.txt")).unwrap().trim(),
            "k2_test_20260101"
        );
        assert!(dest.join("taxo.k2d").is_file() && dest.join("opts.k2d").is_file());
        assert!(!dest.join("hash.k2d").exists() && !root.join("k8.partial").exists());

        // the Kraken2 database is removed: the copy is whole without it, and a second go has nothing to do
        fs::remove_dir_all(&src).unwrap();
        assert_eq!(run(&src), 0);
        // a folder that is not a Kraken2 database: the tool's own words, no half copy left
        fs::remove_dir_all(&dest).unwrap();
        assert_ne!(run(&src), 0);
        let err = fs::read_to_string(job.join(".command.err")).unwrap();
        assert_eq!(
            mlstdb::problem(&err, 1),
            format!("{} is not a Kraken2 database (hash.k2d is missing)", src.display())
        );
        assert!(!has_copy(&root, "k8"));
        let _ = fs::remove_dir_all(&cat.home);
    }
}
