//! The run engine of the Genomics Workflow Microbials (replaces Nextflow in v2):
//! runs a workflow's plan with each tool in its container, resumes stopped or
//! failed runs, publishes results and writes provenance.

pub mod backend;
pub mod diagnose;
pub mod docker;
pub mod executor;
pub mod provenance;
pub mod reports;
pub mod resources;
pub mod runs;
pub mod trial;

pub use backend::{Backend, Docker};
pub use executor::{Executor, Outcome, RunSpec, Status, TaskEvent};
pub use resources::Limits;
