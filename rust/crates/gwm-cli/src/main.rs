//! gwm command line (Genomics Workflow Microbials v2): run workflows, list workflows, tools, databases and runs.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use gwm_core::{compiler, Catalog};
use gwm_engine::runs::{self, StartOptions};
use gwm_engine::{backend, Status, TaskEvent};
use serde_json::Value;

#[derive(Parser)]
#[command(name = "gwm", version, about = "Genomics Workflow Microbials")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a workflow on a folder of paired reads
    Run {
        workflow: String,
        /// folder of paired reads
        input: PathBuf,
        #[arg(long)]
        version: Option<u32>,
        /// name of the run
        #[arg(long)]
        name: Option<String>,
        /// switch a step off (repeat for more)
        #[arg(long = "step-off")]
        steps_off: Vec<String>,
        /// keep the working files after a successful run (troubleshooting)
        #[arg(long)]
        keep_work: bool,
        /// CPU threads for all steps together (0 = all)
        #[arg(long, default_value_t = 0)]
        max_cpus: u32,
        /// GB of memory for all steps together (0 = RAM minus 4 GB)
        #[arg(long, default_value_t = 0)]
        max_memory_gb: u32,
    },
    /// Run a stopped or failed run again; finished steps are reused
    Resume { run_id: String },
    /// Start the app's local server (the GUI talks to it)
    Serve {
        #[arg(long, default_value_t = 8642)]
        port: u16,
        /// also serve the built GUI from this folder (open it in a browser)
        #[arg(long)]
        gui: Option<PathBuf>,
        /// listen on every network address, not only this computer
        #[arg(long)]
        public: bool,
    },
    /// Run a prepared run (the app starts this in its own process)
    #[command(hide = true)]
    Exec { run_id: String },
    /// List runs
    Runs,
    /// Print the run plan of a workflow as JSON
    Plan {
        workflow: String,
        #[arg(long)]
        version: Option<u32>,
    },
    /// List workflows
    Workflows,
    /// List tools
    Tools,
    /// List reference databases
    Db,
}

/// Ctrl+C stops the run (its running steps are killed).
fn stop_on_ctrl_c() -> Result<Arc<AtomicBool>> {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    ctrlc::set_handler(move || {
        eprintln!("stopping: the running steps are being stopped");
        s.store(true, Ordering::Relaxed);
    })?;
    Ok(stop)
}

fn progress(e: &TaskEvent) {
    if e.status != Status::Submitted {
        eprintln!(
            "[{}] {} ({}) {:?}",
            chrono::Local::now().format("%H:%M:%S"),
            e.step,
            e.tag,
            e.status
        );
    }
    if let Some(err) = &e.error {
        eprintln!("{err}");
    }
}

fn finish(run: &Value) {
    let s = |k: &str| run[k].as_str().unwrap_or("").to_string();
    println!("{} {} -> {}", s("id"), s("status"), s("results"));
    if run["status"] != "succeeded" {
        std::process::exit(1);
    }
}

fn main() -> Result<()> {
    if let Some(new) = gwm_core::paths::migrate_home()? {
        eprintln!(
            "Moved your data from ~/.ntg to {} (the app is now called Genomics Workflow Microbials).",
            new.display()
        );
    }
    let cat = Catalog::default();
    match Cli::parse().cmd {
        Cmd::Serve { port, gui, public } => {
            let ip = if public { [0, 0, 0, 0] } else { [127, 0, 0, 1] };
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(gwm_server::serve((ip, port).into(), cat, gui))?;
        }
        Cmd::Exec { run_id } => {
            let stop = stop_on_ctrl_c()?;
            let backend = backend::detect();
            finish(&runs::resume(&cat, &run_id, backend.as_ref(), &stop, &progress)?);
        }
        Cmd::Run {
            workflow,
            input,
            version,
            name,
            steps_off,
            keep_work,
            max_cpus,
            max_memory_gb,
        } => {
            let stop = stop_on_ctrl_c()?;
            let opts = StartOptions {
                name,
                version,
                cleanup: !keep_work,
                steps_off,
                max_cpus,
                max_memory_gb,
                pairing: None,
            };
            let backend = backend::detect();
            finish(&runs::start(
                &cat,
                &workflow,
                &input,
                &opts,
                backend.as_ref(),
                &stop,
                &progress,
            )?);
        }
        Cmd::Resume { run_id } => {
            let stop = stop_on_ctrl_c()?;
            let backend = backend::detect();
            finish(&runs::resume(&cat, &run_id, backend.as_ref(), &stop, &progress)?);
        }
        Cmd::Runs => {
            for r in runs::list(&cat) {
                let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
                println!("{:<50} {:<10} {}", s("id"), s("status"), s("name"));
            }
        }
        Cmd::Plan { workflow, version } => {
            let wf = cat.load_workflow(&workflow, version)?;
            let plan = compiler::plan(&wf, &cat.load_tools()?, &cat.load_databases()?)?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        Cmd::Workflows => {
            for w in cat.list_workflows()? {
                let kind = if w.builtin { "built-in" } else { "user" };
                println!("{:<45} v{:<3} {:<9} {}", w.id, w.latest, kind, w.name);
            }
        }
        Cmd::Tools => {
            for t in cat.load_tools()?.values() {
                println!("{:<22} {:<10} {:<20} {}", t.id, t.version, t.source, t.name);
            }
        }
        Cmd::Db => {
            for db in cat.load_databases()?.values() {
                let state = if db.installed { "installed" } else { "missing  " };
                println!("{state}  {:<22} {} ({} GB)", db.id, db.name, db.size_gb);
            }
        }
    }
    Ok(())
}
