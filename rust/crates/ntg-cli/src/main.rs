//! ntg command line (v2): workflows, tools and databases. Runs and the desktop
//! app come in later crates.

use anyhow::Result;
use clap::{Parser, Subcommand};
use ntg_core::{compiler, Catalog};

#[derive(Parser)]
#[command(name = "ntg", version, about = "NTG Genomics Pipeline")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
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

fn main() -> Result<()> {
    let cat = Catalog::default();
    match Cli::parse().cmd {
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
