//! Core of the Genomics Workflow Microbials: tools, workflows, databases and the
//! workflow compiler. Reads the same files as the Python app (v1):
//!
//!   tools/<id>/tool.yml                 built-in tools
//!   workflows/<id>/workflow.yml         built-in workflows (always locked)
//!   databases/catalog.yml               reference databases the tools can use
//!   $GWM_HOME/tools/<repo>/<id>/        tools installed from tool packs (galaxy_import: made by a Galaxy import)
//!   $GWM_HOME/tool_versions/<id>/<v>/   extra tool versions (Tools page)
//!   $GWM_HOME/workflows/<id>/v<N>.yml   user workflows, one file per saved version

pub mod catalog;
pub mod compiler;
pub mod galaxy;
pub mod paths;
pub mod reads;
pub mod value;

pub use catalog::{Catalog, Database, Step, Tool, Workflow};
pub use compiler::{CompileError, Plan};
