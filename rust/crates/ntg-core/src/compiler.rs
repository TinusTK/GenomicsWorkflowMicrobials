//! Turns a workflow into a run plan: the steps in run order, each with its tool,
//! resolved settings, resources and input wiring. The run engine executes the
//! plan; every run keeps its own copy (plan.json) so it shows exactly what ran.
//! Port of v1 `app/compiler.py` (which wrote Nextflow code instead).
//!
//! Tool command contract (docs/TOOLS.md), unchanged from v1:
//!   SAMPLE, CPUS, MEM_GB          sample id ('all' for all-sample steps), resources
//!   S_<setting>                   every setting, as text ('true'/'false' for booleans)
//!   IN_<input>                    per-sample input file (reads: IN_<input>_1 / _2)
//!   IN_<input>_DIR                all-sample input: folder holding every sample's file
//!   DB_<database>                 database folder
//!   TOOL_DIR                      the tool's own folder (bundled scripts)
//!   write results under out/; notes for provenance go in .ntg_notes ("key: value" lines)

use std::collections::HashSet;

use indexmap::IndexMap;
use serde::Serialize;
use serde_yaml::Value;

use crate::catalog::{step_tool, Database, Resources, Step, Tool, Workflow, ID_RE};
use crate::value;

/// The workflow's own input: the paired reads.
pub const INPUT: &str = "input";

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct CompileError(pub String);

fn err<T>(msg: impl Into<String>) -> Result<T, CompileError> {
    Err(CompileError(msg.into()))
}

/// A workflow step with its tool and settings filled in.
#[derive(Debug, Clone)]
pub struct Resolved<'a> {
    pub id: String,
    pub step: &'a Step,
    pub tool: &'a Tool,
    /// every setting of the tool: the step's value, else the tool's default
    pub settings: IndexMap<String, Value>,
}

/// Check the workflow; return its steps in run order (each after the steps it reads from).
pub fn resolve<'a>(wf: &'a Workflow, tools: &'a IndexMap<String, Tool>) -> Result<Vec<Resolved<'a>>, CompileError> {
    let steps = &wf.steps;
    let mut order: Vec<&str> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();

    fn visit<'s>(
        sid: &'s str,
        steps: &'s IndexMap<String, Step>,
        stack: &mut Vec<&'s str>,
        seen: &mut HashSet<&'s str>,
        order: &mut Vec<&'s str>,
    ) -> Result<(), CompileError> {
        if seen.contains(sid) {
            return Ok(());
        }
        if stack.contains(&sid) {
            let mut path = stack.clone();
            path.push(sid);
            return err(format!("steps form a loop: {}", path.join(" -> ")));
        }
        let step = &steps[sid];
        stack.push(sid);
        for sources in step.inputs.values() {
            for src in sources {
                if src != INPUT {
                    let up = src.split('.').next().unwrap_or("");
                    let Some((up, _)) = steps.get_key_value(up) else {
                        return err(format!("step {sid}: input from unknown step '{up}'"));
                    };
                    visit(up, steps, stack, seen, order)?;
                }
            }
        }
        stack.pop();
        seen.insert(sid);
        order.push(sid);
        Ok(())
    }

    for sid in steps.keys() {
        if !ID_RE.is_match(sid) {
            return err(format!("step id '{sid}': lowercase letters, digits and _ only"));
        }
        visit(sid, steps, &mut Vec::new(), &mut seen, &mut order)?;
    }

    let mut resolved = Vec::new();
    for sid in order {
        let step = &steps[sid];
        let Some(tool) = step_tool(tools, step) else {
            if let Some(t) = tools.get(&step.tool) {
                return err(format!(
                    "step {sid}: {} version {} is not installed (add it on the Tools page, or pick another version)",
                    t.name,
                    step.tool_version.as_deref().unwrap_or("")
                ));
            }
            return err(format!("step {sid}: tool '{}' is not installed", step.tool));
        };
        let mut settings = IndexMap::new();
        for (name, spec) in &tool.settings {
            let v = step
                .settings
                .get(name)
                .cloned()
                .unwrap_or_else(|| default_of(&spec.default));
            settings.insert(name.clone(), v);
        }
        for name in step.settings.keys() {
            if !settings.contains_key(name) {
                return err(format!("step {sid}: tool {} has no setting '{name}'", tool.id));
            }
        }
        for (name, spec) in &tool.inputs {
            let srcs = step.inputs.get(name).map(Vec::as_slice).unwrap_or(&[]);
            if srcs.is_empty() && !spec.optional {
                return err(format!("step {sid}: input '{name}' is not connected"));
            }
            for src in srcs.iter().filter(|s| *s != INPUT) {
                let (up, out) = src.split_once('.').unwrap_or((src, ""));
                let up_tool = step_tool(tools, &steps[up]);
                if !up_tool.is_some_and(|t| t.outputs.contains_key(out)) {
                    return err(format!("step {sid}: step {up} has no output '{out}'"));
                }
            }
        }
        resolved.push(Resolved {
            id: sid.to_string(),
            step,
            tool,
            settings,
        });
    }
    Ok(resolved)
}

/// v1: a setting with no default is the empty text.
fn default_of(v: &Value) -> Value {
    if v.is_null() {
        Value::String(String::new())
    } else {
        v.clone()
    }
}

/// {step: will it run}: a step runs when it is enabled and every required input
/// has at least one source that runs.
pub fn runnable(wf: &Workflow, tools: &IndexMap<String, Tool>) -> Result<IndexMap<String, bool>, CompileError> {
    let mut ok: IndexMap<String, bool> = IndexMap::new();
    for r in resolve(wf, tools)? {
        let mut good = r.step.enabled();
        for (name, spec) in &r.tool.inputs {
            let srcs = r.step.inputs.get(name).map(Vec::as_slice).unwrap_or(&[]);
            let fed = srcs
                .iter()
                .any(|s| s == INPUT || ok.get(s.split('.').next().unwrap()).copied().unwrap_or(false));
            if !spec.optional && !fed {
                good = false;
            }
        }
        ok.insert(r.id, good);
    }
    Ok(ok)
}

// ---------------------------------------------------------------- run plan

/// Everything the engine needs to run a workflow; saved as plan.json in the run.
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub workflow_id: String,
    pub workflow_version: u32,
    pub workflow_name: String,
    pub reads_pattern: String,
    pub steps: Vec<PlanStep>,
    /// database id -> folder name under the databases folder
    pub databases: IndexMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanStep {
    pub id: String,
    pub label: String,
    pub tool_id: String,
    pub tool_name: String,
    pub tool_version: String,
    /// Folder name of the tool inside the run's tools/
    pub tool_bundle: String,
    pub container: String,
    pub per_sample: bool,
    pub enabled: bool,
    /// runs when enabled and its required inputs are fed (before steps_off)
    pub runnable: bool,
    pub resources: Resources,
    /// setting -> text passed as S_<setting>
    pub settings: IndexMap<String, String>,
    /// input -> wiring
    pub inputs: IndexMap<String, PlanInput>,
    /// output -> path pattern (`{sample}` = sample id)
    pub outputs: IndexMap<String, PlanOutput>,
    pub databases: Vec<String>,
    pub files: Vec<String>,
    /// results folder, relative to the run's results/ (`{sample}` = sample id)
    pub publish: String,
    pub versions: IndexMap<String, String>,
    pub command: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanInput {
    pub reads: bool,
    pub optional: bool,
    /// first source whose step runs wins: "input" or "<step>.<output>"
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanOutput {
    pub path: String,
    pub optional: bool,
}

pub fn plan(
    wf: &Workflow,
    tools: &IndexMap<String, Tool>,
    databases: &IndexMap<String, Database>,
) -> Result<Plan, CompileError> {
    let resolved = resolve(wf, tools)?;
    let run = runnable(wf, tools)?;
    let mut used_dbs = IndexMap::new();
    let mut steps = Vec::new();
    for r in &resolved {
        let (tool, step) = (r.tool, r.step);
        for d in &tool.databases {
            let Some(db) = databases.get(d) else {
                return err(format!("step {}: database '{d}' is not in databases/catalog.yml", r.id));
            };
            used_dbs.insert(d.clone(), db.install_dir.clone());
        }
        let per_sample = tool.per_sample();
        let publish = step.publish.clone().unwrap_or_else(|| {
            if per_sample {
                format!("samples/{{sample}}/{}", r.id)
            } else {
                format!("combined/{}", r.id)
            }
        });
        steps.push(PlanStep {
            id: r.id.clone(),
            label: step.label.clone().unwrap_or_else(|| tool.name.clone()),
            tool_id: tool.id.clone(),
            tool_name: tool.name.clone(),
            tool_version: tool.version.clone(),
            tool_bundle: tool.bundle.clone(),
            container: tool.container.clone(),
            per_sample,
            enabled: step.enabled(),
            runnable: run[&r.id],
            resources: tool
                .resources
                .merged(step.resources.as_ref().unwrap_or(&Resources::default())),
            settings: r.settings.iter().map(|(k, v)| (k.clone(), value::to_text(v))).collect(),
            inputs: tool
                .inputs
                .iter()
                .map(|(n, spec)| {
                    (
                        n.clone(),
                        PlanInput {
                            reads: spec.kind == "reads",
                            optional: spec.optional,
                            sources: step.inputs.get(n).cloned().unwrap_or_default(),
                        },
                    )
                })
                .collect(),
            outputs: tool
                .outputs
                .iter()
                .map(|(n, o)| {
                    (
                        n.clone(),
                        PlanOutput {
                            path: o.path.clone(),
                            optional: o.optional,
                        },
                    )
                })
                .collect(),
            databases: tool.databases.clone(),
            files: tool.files.clone(),
            publish,
            versions: tool.version_commands(),
            command: tool.command.clone(),
        });
    }
    used_dbs.sort_keys();
    Ok(Plan {
        workflow_id: wf.id.clone(),
        workflow_version: wf.saved_version.max(1),
        workflow_name: wf.name.clone(),
        reads_pattern: wf.reads_pattern(),
        steps,
        databases: used_dbs,
    })
}
