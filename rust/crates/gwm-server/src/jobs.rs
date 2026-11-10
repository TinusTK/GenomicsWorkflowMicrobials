//! Background jobs (database downloads, ...) the GUI follows by polling
//! `/api/jobs/<id>`, and the database installer (port of v1 `app/databases.py`,
//! now with resumable downloads).

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use gwm_core::Catalog;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: String,
    pub title: String,
    /// running | done | failed
    pub status: String,
    pub log: Vec<String>,
    /// (done, total) bytes or steps
    pub progress: Option<(u64, u64)>,
}

static JOBS: LazyLock<Mutex<HashMap<String, Arc<Mutex<Job>>>>> = LazyLock::new(Default::default);

/// Handle a job's work uses to report.
#[derive(Clone)]
pub struct Reporter(Arc<Mutex<Job>>);

impl Reporter {
    pub fn log(&self, msg: impl Into<String>) {
        self.0.lock().unwrap().log.push(msg.into());
    }
    pub fn progress(&self, done: u64, total: u64) {
        self.0.lock().unwrap().progress = Some((done, total));
    }
}

/// Run `work` in the background; returns the job as it starts.
pub fn spawn(title: &str, work: impl FnOnce(&Reporter) -> Result<()> + Send + 'static) -> Job {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let id = format!("{:x}", nanos & 0xffff_ffff_ffff);
    let job = Arc::new(Mutex::new(Job {
        id: id.clone(),
        title: title.into(),
        status: "running".into(),
        log: vec![],
        progress: None,
    }));
    JOBS.lock().unwrap().insert(id, job.clone());
    let r = Reporter(job.clone());
    std::thread::spawn(move || {
        let result = work(&r);
        let mut j = r.0.lock().unwrap();
        match result {
            Ok(()) => j.status = "done".into(),
            Err(e) => {
                j.log.push(format!("Error: {e:#}"));
                j.status = "failed".into();
            }
        }
    });
    let snapshot = job.lock().unwrap().clone();
    snapshot
}

pub fn get(id: &str) -> Option<Job> {
    JOBS.lock().unwrap().get(id).map(|j| j.lock().unwrap().clone())
}

/// Download and unpack a reference database. A broken download picks up where it stopped.
pub fn install_database(cat: &Catalog, db_id: &str, r: &Reporter) -> Result<()> {
    let dbs = cat.load_databases()?;
    let db = dbs.get(db_id).with_context(|| format!("unknown database {db_id}"))?;
    if db.installed {
        r.log(format!("{} is already installed.", db.name));
        return Ok(());
    }
    let dest = &db.path;
    let parent = dest.parent().context("database folder")?;
    fs::create_dir_all(parent)?;
    let name = dest.file_name().unwrap().to_string_lossy().into_owned();
    let tmp = parent.join(format!("{name}.download"));
    download(&db.url, &tmp, r).with_context(|| format!("downloading {}", db.url))?;
    if db.format != "tar.gz" {
        bail!("unknown database format {}", db.format);
    }
    r.log("Unpacking (this can take a few minutes)");
    let work = parent.join(format!("{name}.partial"));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(&tmp)?));
    archive.set_overwrite(true);
    for entry in archive.entries()? {
        let mut entry = entry?;
        // never outside the folder (unpack_in refuses ../ and absolute paths)
        entry.unpack_in(&work)?;
    }
    fs::rename(&work, dest)?;
    fs::remove_file(&tmp).ok();
    if !dest.join(&db.check_file).exists() {
        bail!(
            "unpacked, but {} is missing: the download may be damaged",
            db.check_file
        );
    }
    r.log(format!("Installed {} at {}", db.name, dest.display()));
    Ok(())
}

fn download(url: &str, to: &Path, r: &Reporter) -> Result<()> {
    let have = fs::metadata(to).map(|m| m.len()).unwrap_or(0);
    let mut req = ureq::get(url);
    if have > 0 {
        req = req.set("Range", &format!("bytes={have}-"));
        r.log(format!("Carrying on from {:.1} GB", have as f64 / 1e9));
    } else {
        r.log(format!("Downloading {url}"));
    }
    let resp = req.call()?;
    let resumed = resp.status() == 206;
    let len: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let start = if resumed { have } else { 0 };
    let total = start + len;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(to)?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = start;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        r.progress(done, total);
    }
    if total > 0 && done < total {
        bail!(
            "download stopped at {:.1} of {:.1} GB; install again to carry on",
            done as f64 / 1e9,
            total as f64 / 1e9
        );
    }
    Ok(())
}
