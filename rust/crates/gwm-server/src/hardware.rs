//! This computer's specs, live use for the top bar, and a workflow's needs
//! turned into plain warnings (port of v1 `app/hardware.py`).

use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use gwm_core::{catalog, compiler, paths, Catalog, Workflow};
use serde_json::{json, Value};
use sysinfo::{Disks, System};

static SYS: Mutex<Option<System>> = Mutex::new(None);

fn gb(bytes: u64) -> f64 {
    (bytes as f64 / 1_073_741_824.0 * 10.0).round() / 10.0
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// Free and total GB of the disk holding `path` (or its nearest existing parent).
fn disk(path: &Path) -> (Option<f64>, Option<f64>) {
    let mut p = path.to_path_buf();
    while !p.exists() && p.pop() {}
    let disks = Disks::new_with_refreshed_list();
    let best = disks
        .list()
        .iter()
        .filter(|d| p.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len());
    match best {
        Some(d) => (Some(gb(d.available_space())), Some(gb(d.total_space()))),
        None => (None, None),
    }
}

fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version").is_ok_and(|v| v.to_lowercase().contains("microsoft"))
}

pub fn check(cat: &Catalog) -> Value {
    let mut sys = System::new();
    sys.refresh_memory();
    let docker = run("docker", &["info", "--format", "{{.ServerVersion}}"]);
    let docker_error = if docker.is_some() {
        None
    } else if run("docker", &["--version"]).is_some() {
        Some("Docker is installed but not running".to_string())
    } else {
        Some("Docker is not installed".to_string())
    };
    json!({
        "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "memory_gb": gb(sys.total_memory()),
        "memory_available_gb": gb(sys.available_memory()),
        "wsl": is_wsl(),
        "os": System::long_os_version(),
        "docker": docker,
        "docker_error": docker_error,
        "apptainer": run("apptainer", &["--version"]),
        "disk_free_gb": {
            "work": disk(&cat.home.join("work")).0,
            "databases": disk(&paths::db_dir(&cat.home)).0,
        },
        "app_version": env!("CARGO_PKG_VERSION"),
        "runs_folder": cat.runs,
        "data_folder": cat.home,
        "work_folder": cat.home.join("work"),
        "databases_folder": paths::db_dir(&cat.home),
    })
}

/// Live use for the top bar (cheap: no docker calls).
pub fn usage(cat: &Catalog) -> Value {
    let mut guard = SYS.lock().unwrap();
    let sys = guard.get_or_insert_with(|| {
        let mut s = System::new();
        s.refresh_cpu_usage();
        s
    });
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    let cpus = sys.cpus();
    let ghz = if cpus.is_empty() || cpus[0].frequency() == 0 {
        None
    } else {
        Some((cpus.iter().map(|c| c.frequency() as f64).sum::<f64>() / cpus.len() as f64 / 10.0).round() / 100.0)
    };
    let (free, total) = disk(&cat.home.join("work"));
    json!({
        "cpus": cpus.len(),
        "cpu_percent": (sys.global_cpu_usage() as f64 * 10.0).round() / 10.0,
        "cpu_ghz": ghz,
        "memory_gb": gb(sys.total_memory()),
        "memory_used_gb": gb(sys.used_memory()),
        "disk_free_gb": free,
        "disk_total_gb": total,
    })
}

/// [{level: error|warning|info, step, message}] for running `wf` here.
pub fn warnings(cat: &Catalog, wf: &Workflow, hw: &Value, n_samples: Option<usize>) -> Vec<Value> {
    let mut out = Vec::new();
    let mut add = |level: &str, msg: String, step: Option<&str>| {
        out.push(json!({"level": level, "step": step, "message": msg}));
    };
    if hw["docker"].is_null() && hw["apptainer"].is_null() {
        let why = hw["docker_error"].as_str().unwrap_or("no container engine");
        add(
            "error",
            format!("The tools can't start: {why}. Open Settings to set it up."),
            None,
        );
    }
    let cpus = hw["cpus"].as_u64().unwrap_or(0);
    if cpus > 0 && cpus < 4 {
        add("warning", format!("Only {cpus} CPU threads: runs will be slow."), None);
    }
    let mem = hw["memory_gb"].as_f64().unwrap_or(0.0);
    let (Ok(tools), Ok(dbs)) = (cat.load_tools(), cat.load_databases()) else {
        return out;
    };
    let will_run = compiler::runnable(wf, &tools).unwrap_or_default();
    let mut disk_per_sample = 0.0;
    for (sid, step) in &wf.steps {
        if !will_run.get(sid).copied().unwrap_or(step.enabled()) {
            continue;
        }
        let Some(tool) = catalog::step_tool(&tools, step) else {
            add("error", format!("Tool '{}' is not installed.", step.tool), Some(sid));
            continue;
        };
        let label = format!(
            "{} ({})",
            step.label.clone().unwrap_or_else(|| tool.name.clone()),
            tool.name
        );
        let req = tool.extra.get("requirements");
        let num = |k: &str| req.and_then(|r| r.get(k)).and_then(|v| v.as_f64()).unwrap_or(0.0);
        if num("memory_gb") > mem {
            add(
                "error",
                format!(
                    "{label} needs at least {} GB RAM; this computer has {mem:.0} GB.",
                    num("memory_gb")
                ),
                Some(sid),
            );
        } else if num("recommended_memory_gb") > mem {
            let note = req.and_then(|r| r.get("note")).and_then(|v| v.as_str()).unwrap_or("");
            add(
                "warning",
                format!(
                    "{label} works best with {} GB RAM; this computer has {mem:.0} GB. {note}",
                    num("recommended_memory_gb")
                )
                .trim()
                .to_string(),
                Some(sid),
            );
        }
        let full = num("full_tree_memory_gb");
        if full > 0.0 && mem < full {
            let tree = step.settings.get("tree").and_then(|v| v.as_str()).unwrap_or("auto");
            if tree == "full" {
                add("error", format!("{label}: the full tree needs about {full} GB RAM; this computer has {mem:.0} GB. Set Reference tree to Auto or Reduced."), Some(sid));
            } else if tree == "auto" {
                add("info", format!("{label}: with {mem:.0} GB RAM, Auto uses the reduced tree (Galaxy used the full tree; results can differ slightly)."), Some(sid));
            }
        }
        for d in &compiler::step_databases(tool, step).0 {
            match dbs.get(d) {
                Some(db) if db.installed => {
                    // it is loaded into memory whole
                    let needs = db.extra.get("memory_gb").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    if needs > mem {
                        add("error", format!("{label}: the {} database needs about {needs:.0} GB RAM; this computer has {mem:.0} GB. Pick a smaller database in the step's settings.", db.name), Some(sid));
                    }
                }
                Some(db) => add("error", format!("{label} needs the {} database, which is not installed ({} GB download). Install it on the Databases page.", db.name, db.size_gb), Some(sid)),
                None => add("error", format!("{label} needs the database '{d}', which this app doesn't know."), Some(sid)),
            }
        }
        disk_per_sample += if num("disk_gb_per_sample") > 0.0 {
            num("disk_gb_per_sample")
        } else {
            0.2
        };
    }
    if let Some(n) = n_samples.filter(|n| *n > 0) {
        let need = (disk_per_sample * n as f64 * 1.5 * 10.0).round() / 10.0;
        if let Some(free) = hw["disk_free_gb"]["work"].as_f64() {
            if free < need {
                add(
                    "error",
                    format!("About {need} GB of free disk is needed for {n} samples; only {free} GB is free."),
                    None,
                );
            } else if free < need * 3.0 {
                add(
                    "warning",
                    format!("Disk space is getting low: {free} GB free; {n} samples need about {need} GB."),
                    None,
                );
            }
        }
    }
    if hw["wsl"] == true && mem < 32.0 {
        add("info", format!("The Linux side (WSL) has {mem:.0} GB RAM. Windows gives it half its memory by default; Settings shows how to raise it."), None);
    }
    out
}
