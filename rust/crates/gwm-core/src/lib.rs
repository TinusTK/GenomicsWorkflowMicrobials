//! Core of the Genomics Workflow Microbials: tools, workflows, databases and the
//! workflow compiler. The files it reads:
//!
//!   tools/<id>/tool.yml                 built-in tools
//!   workflows/<id>/workflow.yml         built-in workflows (always locked)
//!   databases/catalog.yml               reference databases the tools can use
//!   $GWM_HOME/tools/<source>/<id>/      tools added on this computer (github: from a GitHub link, galaxy_import: by a Galaxy import)
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
