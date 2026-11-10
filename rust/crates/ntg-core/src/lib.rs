//! Core of the NTG Genomics Pipeline: tools, workflows, databases and the
//! workflow compiler. Reads the same files as the Python app (v1):
//!
//!   tools/<id>/tool.yml                 built-in tools
//!   workflows/<id>/workflow.yml         built-in workflows (always locked)
//!   databases/catalog.yml               reference databases the tools can use
//!   $NTG_HOME/tools/<repo>/<id>/        tools installed from tool packs
//!   $NTG_HOME/tool_versions/<id>/<v>/   extra tool versions (Tools page)
//!   $NTG_HOME/workflows/<id>/v<N>.yml   user workflows, one file per saved version

pub mod catalog;
pub mod compiler;
pub mod paths;
pub mod value;

pub use catalog::{Catalog, Database, Step, Tool, Workflow};
pub use compiler::{CompileError, Plan};
