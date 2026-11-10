//! Imports a Galaxy workflow file (`.ga`) as a workflow of this app.
//!
//! [`analyze`] reads the file and decides, per Galaxy step, what it becomes.
//! It changes nothing and needs no network, so the GUI can show the list first:
//!
//!   builtin      a tool of this app at the same version (matched through the
//!                `galaxy:` block in its tool.yml)
//!   new_version  a tool of this app at another version: that version is added
//!                beside the built-in one (`tool_versions/`), from BioContainers
//!   new_tool     a tool this app does not know: a draft tool is made with its
//!                pinned container, inputs, outputs and settings; its command
//!                is left for the user (a .ga file has no command lines)
//!   summary      Galaxy text and collection steps (cut, join, collapse ...)
//!                that the app's summary tool covers
//!   input        the reads
//!   left_out     nothing in the app matches
//!
//! [`apply`] then adds the versions and tools and saves the workflow.
//!
//! The `galaxy:` block of a tool.yml:
//!
//!   galaxy:
//!     tool: iuc/shovill/shovill            # Tool Shed owner/repository/tool id
//!     any_version: true                    # optional: stands in for every Galaxy version
//!     inputs: {"library|input1": reads}    # Galaxy input name -> this tool's input
//!     outputs: {contigs: contigs}          # Galaxy output name -> this tool's output
//!     settings:                            # this tool's setting <- place in Galaxy's tool_state
//!       depth: adv.depth
//!       nocorr: {from: adv.keep_files.nocorr, map: {no_correction: true}, default: false}
//!       raw: {from: raw_outputs, has: summary}     # true when the list holds the value
//!     step: {publish: summary}             # optional: added to every imported step
//!     folds: [bgruening/text_processing]   # summary tools: Tool Shed repositories covered
//!     sources: {mlst: {tool: mlst, output: report}}   # summary tools: what feeds each input
//!                                          # (`always: true`: also with no Galaxy text step after it)

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{anyhow, bail, Context, Result};
use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use serde_yaml::{Mapping, Value};

use crate::catalog::{dump_yaml, load_yaml, Catalog, Step, Tool, Workflow, ID_RE};
use crate::compiler::{self, INPUT};
use crate::value::to_text;

/// Folder under `<home>/tools/` holding the tools made by an import.
pub const IMPORT_SOURCE: &str = "galaxy_import";
const QUAY: &str = "quay.io/biocontainers/";
const DRAFT_COMMAND: &str = "\
# Imported from Galaxy. A Galaxy workflow file has no command lines, so write
# this tool's command on the Tools page (open the tool, then Edit).
echo 'This tool was imported from Galaxy and has no command yet.' >&2
exit 1
";

// ---------------------------------------------------------------- the galaxy: block

#[derive(Debug, Clone, Default, Deserialize)]
struct GalaxyMap {
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    any_version: bool,
    #[serde(default)]
    inputs: IndexMap<String, String>,
    #[serde(default)]
    outputs: IndexMap<String, String>,
    #[serde(default)]
    settings: IndexMap<String, SettingMap>,
    #[serde(default)]
    step: Mapping,
    #[serde(default)]
    folds: Vec<String>,
    #[serde(default)]
    sources: IndexMap<String, SourceSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum SettingMap {
    Path(String),
    Full {
        from: String,
        /// Galaxy value (as text) -> this app's value, or `{value, galaxy, note}`
        #[serde(default)]
        map: Mapping,
        /// true when Galaxy's list holds this value
        #[serde(default)]
        has: Option<String>,
        /// used when `map` has no entry for the Galaxy value
        #[serde(default)]
        default: Option<Value>,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct SourceSpec {
    tool: String,
    output: String,
    #[serde(default)]
    settings: IndexMap<String, Value>,
    /// connect it whenever such a step is there, also when Galaxy puts no text step after it
    #[serde(default)]
    always: bool,
}

fn galaxy_map(tool: &Tool) -> Option<GalaxyMap> {
    let v = tool.extra.get("galaxy")?;
    serde_yaml::from_value(v.clone()).ok()
}

// ---------------------------------------------------------------- what an import gives

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    /// step number in the Galaxy file
    pub galaxy_step: u32,
    /// Galaxy's label, else the tool's name there
    pub name: String,
    pub galaxy_tool: String,
    pub galaxy_version: String,
    /// builtin | new_version | new_tool | summary | input | left_out
    pub action: String,
    /// the step it became in this app's workflow
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    pub note: String,
}

/// Another version of a tool the app has, to add from BioContainers.
#[derive(Debug, Clone, Serialize)]
pub struct NewVersion {
    pub tool: String,
    pub version: String,
}

/// Where a Galaxy tool comes from in the Tool Shed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ShedRef {
    pub owner: String,
    pub name: String,
    pub changeset_revision: String,
    pub tool_id: String,
}

/// A tool the app does not know, made from what the Galaxy file says about it.
#[derive(Debug, Clone, Serialize)]
pub struct DraftTool {
    pub id: String,
    /// the tool.yml to write
    pub yaml: Mapping,
    pub shed: Option<ShedRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Import {
    pub workflow: Workflow,
    pub steps: Vec<StepReport>,
    pub new_versions: Vec<NewVersion>,
    pub new_tools: Vec<DraftTool>,
}

// ---------------------------------------------------------------- reading the .ga file

struct GStep {
    n: u32,
    kind: String,
    /// `owner/repo/tool` for Tool Shed tools, else Galaxy's own tool id
    key: String,
    /// `owner/repo` for Tool Shed tools
    repo: Option<String>,
    short_id: String,
    version: String,
    label: Option<String>,
    name: String,
    state: Json,
    /// (input name, upstream step, its output)
    conns: Vec<(String, u32, String)>,
    /// (output name, Galaxy datatype)
    outputs: Vec<(String, String)>,
    changeset: String,
}

static GALAXY_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\+galaxy\d+$").unwrap());

/// `1.4.2+galaxy1` -> `1.4.2`: the tool's own version without the wrapper's.
pub fn tool_version(galaxy_version: &str) -> String {
    GALAXY_SUFFIX.replace(galaxy_version, "").into_owned()
}

fn slug(text: &str) -> String {
    let mut s = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('_') && !s.is_empty() {
            s.push('_');
        }
    }
    let s = s.trim_end_matches('_').to_string();
    match s.chars().next() {
        Some(c) if c.is_ascii_lowercase() => s,
        Some(_) => format!("s_{s}"),
        None => "step".into(),
    }
}

fn unique(base: &str, taken: &dyn Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|c| !taken(c))
        .unwrap_or_else(|| base.to_string())
}

/// Galaxy keeps `tool_state` as JSON text, and older files nest JSON text inside it.
fn unpack(v: Json) -> Json {
    match v {
        Json::String(s) if s.starts_with('{') || s.starts_with('[') => match serde_json::from_str::<Json>(&s) {
            Ok(inner) => unpack(inner),
            Err(_) => Json::String(s),
        },
        Json::Object(o) => Json::Object(o.into_iter().map(|(k, v)| (k, unpack(v))).collect()),
        Json::Array(a) => Json::Array(a.into_iter().map(unpack).collect()),
        other => other,
    }
}

fn read_steps(ga: &Json) -> Result<Vec<GStep>> {
    let steps = ga
        .get("steps")
        .and_then(Json::as_object)
        .ok_or_else(|| anyhow!("this file has no steps: is it a Galaxy workflow (.ga) file?"))?;
    let text = |v: &Json, k: &str| v.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
    let mut out = Vec::new();
    for (k, s) in steps {
        let n: u32 = s
            .get("id")
            .and_then(Json::as_u64)
            .map(|n| n as u32)
            .or_else(|| k.parse().ok())
            .ok_or_else(|| anyhow!("step {k}: no step number"))?;
        let tool_id = text(s, "tool_id");
        let parts: Vec<&str> = tool_id.split('/').collect();
        let (key, repo, short_id) = match parts.iter().position(|p| *p == "repos") {
            Some(i) if parts.len() >= i + 4 => (
                format!("{}/{}/{}", parts[i + 1], parts[i + 2], parts[i + 3]),
                Some(format!("{}/{}", parts[i + 1], parts[i + 2])),
                parts[i + 3].to_string(),
            ),
            _ => (tool_id.clone(), None, tool_id.clone()),
        };
        let mut conns = Vec::new();
        if let Some(ic) = s.get("input_connections").and_then(Json::as_object) {
            for (name, c) in ic {
                let list = match c {
                    Json::Array(a) => a.clone(),
                    other => vec![other.clone()],
                };
                for c in list {
                    if let Some(up) = c.get("id").and_then(Json::as_u64) {
                        conns.push((name.clone(), up as u32, text(&c, "output_name")));
                    }
                }
            }
        }
        let outputs = s
            .get("outputs")
            .and_then(Json::as_array)
            .map(|a| a.iter().map(|o| (text(o, "name"), text(o, "type"))).collect())
            .unwrap_or_default();
        out.push(GStep {
            n,
            kind: text(s, "type"),
            key,
            repo,
            short_id,
            version: text(s, "tool_version"),
            label: s
                .get("label")
                .and_then(Json::as_str)
                .filter(|l| !l.trim().is_empty())
                .map(str::to_string),
            name: text(s, "name"),
            state: unpack(s.get("tool_state").cloned().unwrap_or(Json::Null)),
            conns,
            outputs,
            changeset: s
                .get("tool_shed_repository")
                .map(|r| text(r, "changeset_revision"))
                .unwrap_or_default(),
        });
    }
    out.sort_by_key(|s| s.n);
    Ok(out)
}

/// The value at `a.b.c` in a step's tool_state.
fn at<'a>(state: &'a Json, path: &str) -> Option<&'a Json> {
    let mut v = state;
    for part in path.split('.') {
        v = v.get(part)?;
    }
    Some(v)
}

fn json_text(v: &Json) -> String {
    match v {
        Json::Null => String::new(),
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A Galaxy value as a setting of this kind; None = leave the tool's default.
fn coerce(kind: &str, v: &Json) -> Option<Value> {
    let text = json_text(v);
    match kind {
        _ if v.is_null() => None,
        "boolean" => match text.to_lowercase().as_str() {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            _ => None,
        },
        "integer" => text
            .parse::<i64>()
            .ok()
            .or_else(|| text.parse::<f64>().ok().filter(|f| f.fract() == 0.0).map(|f| f as i64))
            .map(|n| Value::Number(n.into())),
        // keeps Galaxy's own writing: "2" stays 2, "98.0" stays 98.0
        "number" => text
            .parse::<i64>()
            .map(|n| Value::Number(n.into()))
            .ok()
            .or_else(|| text.parse::<f64>().ok().map(|f| Value::Number(f.into()))),
        _ if v.is_object() || v.is_array() => None,
        _ => Some(Value::String(text)),
    }
}

/// One setting of an imported step: (value, Galaxy's own value and a note when they differ).
fn setting_value(kind: &str, map: &SettingMap, state: &Json) -> Option<(Value, Option<(Value, String)>)> {
    match map {
        SettingMap::Path(p) => coerce(kind, at(state, p)?).map(|v| (v, None)),
        SettingMap::Full {
            from,
            map,
            has,
            default,
        } => {
            let v = at(state, from);
            if let Some(item) = has {
                let held = match v {
                    Some(Json::Array(a)) => a.iter().any(|x| json_text(x) == *item),
                    Some(Json::String(s)) => s.split(',').any(|x| x.trim() == item),
                    _ => false,
                };
                return Some((Value::Bool(held), None));
            }
            let v = v.filter(|v| !v.is_null())?;
            let text = json_text(v);
            let Some((_, to)) = map.iter().find(|(k, _)| to_text(k) == text) else {
                return match default {
                    Some(d) => Some((d.clone(), None)),
                    None => coerce(kind, v).map(|v| (v, None)),
                };
            };
            match to.get("value") {
                Some(value) if to.is_mapping() => {
                    let note = to.get("note").map(to_text).unwrap_or_default();
                    let galaxy = to.get("galaxy").cloned().unwrap_or(Value::String(text));
                    Some((value.clone(), Some((galaxy, note))))
                }
                _ => Some((to.clone(), None)),
            }
        }
    }
}

// ---------------------------------------------------------------- analyze

enum Kind {
    Input,
    /// a tool of this app (by id); `draft` = made by this import
    Tool {
        tool: String,
        draft: bool,
    },
    /// a Galaxy text or collection step
    Utility,
    LeftOut(String),
}

/// `3, 4, 5, 9` -> `3-5, 9`
fn ranges(ns: &BTreeSet<u32>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut run: Option<(u32, u32)> = None;
    let flush = |r: Option<(u32, u32)>, out: &mut Vec<String>| {
        if let Some((a, b)) = r {
            out.push(if a == b { a.to_string() } else { format!("{a}-{b}") });
        }
    };
    for &n in ns {
        run = match run {
            Some((a, b)) if n == b + 1 => Some((a, n)),
            other => {
                flush(other, &mut out);
                Some((n, n))
            }
        };
    }
    flush(run, &mut out);
    out.join(", ")
}

fn galaxy_kind(datatype: &str) -> &str {
    match datatype {
        "tabular" | "tsv" => "tsv",
        "fasta" | "fasta.gz" => "fasta",
        "html" => "html",
        "json" => "json",
        "csv" => "csv",
        "txt" | "input" | "" => "txt",
        other => other,
    }
}

/// Every plain value in a tool_state, as `section_name` -> (path, value).
fn flatten(state: &Json, path: &str, out: &mut Vec<(String, Json)>) {
    let Some(o) = state.as_object() else { return };
    for (k, v) in o {
        if k.starts_with("__") {
            continue;
        }
        let p = if path.is_empty() {
            k.clone()
        } else {
            format!("{path}.{k}")
        };
        match v {
            Json::Object(inner) if inner.contains_key("__class__") => {}
            Json::Object(_) => flatten(v, &p, out),
            Json::Array(_) => {}
            _ => out.push((p, v.clone())),
        }
    }
}

/// A draft tool.yml for a Galaxy tool the app does not know.
fn draft_tool(id: &str, g: &GStep, input_kinds: &[(String, String, String, bool)]) -> Mapping {
    let y = |s: &str| Value::String(s.to_string());
    let mut m = Mapping::new();
    m.insert(y("id"), y(id));
    m.insert(y("name"), y(if g.name.is_empty() { &g.short_id } else { &g.name }));
    m.insert(y("version"), y(&tool_version(&g.version)));
    m.insert(y("category"), y("Imported from Galaxy"));
    m.insert(
        y("description"),
        y("Imported from a Galaxy workflow. Its container, inputs, outputs and settings come from Galaxy; its command still has to be written."),
    );
    m.insert(y("draft"), Value::Bool(true));
    m.insert(y("container"), y(""));
    m.insert(y("scope"), y("per_sample"));
    let mut g_inputs = Mapping::new();
    let mut inputs = Mapping::new();
    for (galaxy_name, name, kind, optional) in input_kinds {
        let mut spec = Mapping::new();
        spec.insert(y("type"), y(kind));
        if *optional {
            spec.insert(y("optional"), Value::Bool(true));
        }
        spec.insert(y("description"), y(&format!("Galaxy input {galaxy_name}.")));
        inputs.insert(y(name), Value::Mapping(spec));
        g_inputs.insert(y(galaxy_name), y(name));
    }
    m.insert(y("inputs"), Value::Mapping(inputs));
    let mut g_outputs = Mapping::new();
    let mut outputs = Mapping::new();
    for (name, datatype) in &g.outputs {
        let kind = galaxy_kind(datatype);
        let out = slug(name);
        let mut spec = Mapping::new();
        spec.insert(y("type"), y(kind));
        spec.insert(y("path"), y(&format!("out/{{sample}}.{out}.{kind}")));
        spec.insert(y("optional"), Value::Bool(true));
        spec.insert(y("description"), y(&format!("Galaxy output {name}.")));
        outputs.insert(y(&out), Value::Mapping(spec));
        g_outputs.insert(y(name), y(&out));
    }
    m.insert(y("outputs"), Value::Mapping(outputs));
    let mut flat = Vec::new();
    flatten(&g.state, "", &mut flat);
    let mut g_settings = Mapping::new();
    let mut settings = Mapping::new();
    for (path, v) in flat {
        let name = slug(&path.replace('.', "_"));
        let mut spec = Mapping::new();
        spec.insert(y("label"), y(path.rsplit('.').next().unwrap_or(&path)));
        let (kind, default) = match &v {
            Json::Bool(b) => ("boolean", Value::Bool(*b)),
            other => ("string", y(&json_text(other))),
        };
        spec.insert(y("type"), y(kind));
        spec.insert(y("default"), default);
        spec.insert(y("description"), y(&format!("Galaxy setting {path}.")));
        settings.insert(y(&name), Value::Mapping(spec));
        g_settings.insert(y(&name), y(&path));
    }
    m.insert(y("settings"), Value::Mapping(settings));
    m.insert(y("version_command"), y("echo unknown"));
    let mut gal = Mapping::new();
    gal.insert(y("tool"), y(&g.key));
    gal.insert(y("inputs"), Value::Mapping(g_inputs));
    gal.insert(y("outputs"), Value::Mapping(g_outputs));
    gal.insert(y("settings"), Value::Mapping(g_settings));
    m.insert(y("galaxy"), Value::Mapping(gal));
    m.insert(y("command"), y(DRAFT_COMMAND));
    m
}

fn tool_from_yaml(yaml: &Mapping) -> Result<Tool> {
    let mut t: Tool = serde_yaml::from_value(Value::Mapping(yaml.clone()))?;
    t.source = IMPORT_SOURCE.into();
    t.bundle = t.id.clone();
    Ok(t)
}

/// Read a Galaxy workflow and work out the workflow, tool versions and tools it
/// becomes here. `taken` says whether a workflow id is in use.
pub fn analyze(ga_text: &str, tools: &IndexMap<String, Tool>, taken: &dyn Fn(&str) -> bool) -> Result<Import> {
    let ga: Json = serde_json::from_str(ga_text).context("this is not a Galaxy workflow (.ga) file")?;
    if ga.get("a_galaxy_workflow").is_none() {
        bail!("this is not a Galaxy workflow (.ga) file");
    }
    let gsteps = read_steps(&ga)?;
    let by_n: HashMap<u32, &GStep> = gsteps.iter().map(|g| (g.n, g)).collect();

    // tools as the import sees them: the app's, plus the drafts it makes
    let mut all: IndexMap<String, Tool> = tools.clone();
    let maps = |all: &IndexMap<String, Tool>| -> HashMap<String, GalaxyMap> {
        all.iter()
            .filter_map(|(id, t)| Some((id.clone(), galaxy_map(t)?)))
            .collect()
    };
    let mut gmaps = maps(&all);
    let by_key: HashMap<String, String> = gmaps
        .iter()
        .filter_map(|(id, m)| Some((m.tool.clone()?, id.clone())))
        .collect();
    let folds: HashSet<String> = gmaps.values().flat_map(|m| m.folds.iter().cloned()).collect();

    // ---- what each Galaxy step is
    let mut kinds: HashMap<u32, Kind> = HashMap::new();
    let mut drafts: IndexMap<String, DraftTool> = IndexMap::new();
    let mut draft_ids: HashMap<String, String> = HashMap::new();
    for g in &gsteps {
        let kind = match g.kind.as_str() {
            "data_input" | "data_collection_input" => Kind::Input,
            "tool" => {
                if let Some(id) = by_key.get(&g.key) {
                    Kind::Tool {
                        tool: id.clone(),
                        draft: false,
                    }
                } else if g.repo.as_deref().is_none_or(|r| folds.contains(r)) {
                    Kind::Utility
                } else {
                    let id = match draft_ids.get(&g.key) {
                        Some(id) => id.clone(),
                        None => {
                            let id = unique(&slug(&g.short_id), &|c| all.contains_key(c));
                            // inputs: named after Galaxy's, typed by what feeds them
                            let mut ins: Vec<(String, String, String, bool)> = Vec::new();
                            for (name, up, out) in &g.conns {
                                let short = slug(name.rsplit('|').next().unwrap_or(name));
                                if ins.iter().any(|i| i.0 == *name) {
                                    continue;
                                }
                                let (kind, optional) = match (by_n.get(up), kinds.get(up)) {
                                    (_, Some(Kind::Input)) => ("reads".to_string(), false),
                                    (Some(_), Some(Kind::Tool { tool, .. })) => {
                                        let k = gmaps
                                            .get(tool)
                                            .and_then(|m| m.outputs.get(out))
                                            .and_then(|o| all[tool].outputs.get(o))
                                            .map(|o| o.kind.clone());
                                        (k.clone().unwrap_or_else(|| "any".into()), k.is_none())
                                    }
                                    _ => ("any".to_string(), true),
                                };
                                ins.push((name.clone(), short, kind, optional));
                            }
                            let yaml = draft_tool(&id, g, &ins);
                            let tool = tool_from_yaml(&yaml)?;
                            all.insert(id.clone(), tool);
                            gmaps = maps(&all);
                            let shed = g.repo.as_ref().map(|r| {
                                let (owner, name) = r.split_once('/').unwrap_or((r, ""));
                                ShedRef {
                                    owner: owner.into(),
                                    name: name.into(),
                                    changeset_revision: g.changeset.clone(),
                                    tool_id: g.short_id.clone(),
                                }
                            });
                            drafts.insert(
                                id.clone(),
                                DraftTool {
                                    id: id.clone(),
                                    yaml,
                                    shed,
                                },
                            );
                            draft_ids.insert(g.key.clone(), id.clone());
                            id
                        }
                    };
                    Kind::Tool { tool: id, draft: true }
                }
            }
            "subworkflow" => Kind::LeftOut("A workflow inside the workflow: import it on its own.".into()),
            other => Kind::LeftOut(format!("A Galaxy '{other}' step has no match in this app.")),
        };
        kinds.insert(g.n, kind);
    }

    // ---- step ids: the tool's id, or Galaxy's label when a tool is used more than once
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for k in kinds.values() {
        if let Kind::Tool { tool, .. } = k {
            *uses.entry(tool.as_str()).or_default() += 1;
        }
    }
    let mut step_ids: HashMap<u32, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for g in &gsteps {
        let Some(Kind::Tool { tool, .. }) = kinds.get(&g.n) else {
            continue;
        };
        let base = match (&g.label, uses[tool.as_str()]) {
            (Some(l), n) if n > 1 => slug(l),
            _ => tool.clone(),
        };
        let id = unique(&base, &|c| used.contains(c));
        used.insert(id.clone());
        step_ids.insert(g.n, id);
    }

    // ---- the steps
    let mut steps: IndexMap<String, Step> = IndexMap::new();
    let mut notes: HashMap<u32, Vec<String>> = HashMap::new();
    let mut new_versions: Vec<NewVersion> = Vec::new();
    let mut actions: HashMap<u32, (&str, Option<String>)> = HashMap::new();
    for g in &gsteps {
        let Some(Kind::Tool { tool: tool_id, draft }) = kinds.get(&g.n) else {
            continue;
        };
        let tool = &all[tool_id];
        let gmap = gmaps.get(tool_id).cloned().unwrap_or_default();
        let sid = step_ids[&g.n].clone();
        let mut step = Step {
            tool: tool_id.clone(),
            label: Some(g.label.clone().unwrap_or_else(|| tool.name.clone())),
            ..Step::default()
        };
        step.extra.insert("galaxy_step".into(), Value::Number(g.n.into()));
        for (k, v) in &gmap.step {
            match k.as_str() {
                Some("publish") => step.publish = v.as_str().map(str::to_string),
                _ => {
                    step.extra.insert(k.clone(), v.clone());
                }
            }
        }

        // version
        let want = tool_version(&g.version);
        let action = if *draft {
            "new_tool"
        } else if gmap.any_version || want == tool.version || want.is_empty() {
            "builtin"
        } else if tool.variants.contains_key(&want) {
            step.tool_version = Some(want.clone());
            "builtin"
        } else {
            step.tool_version = Some(want.clone());
            if !new_versions.iter().any(|v| v.tool == *tool_id && v.version == want) {
                new_versions.push(NewVersion {
                    tool: tool_id.clone(),
                    version: want.clone(),
                });
            }
            "new_version"
        };
        actions.insert(g.n, (action, step.tool_version.clone()));

        // settings
        let mut galaxy_values = Mapping::new();
        let mut galaxy_notes: Vec<String> = Vec::new();
        for (name, smap) in &gmap.settings {
            let Some(spec) = tool.settings.get(name) else { continue };
            if let Some((v, differs)) = setting_value(&spec.kind, smap, &g.state) {
                step.settings.insert(name.clone(), v);
                if let Some((galaxy, note)) = differs {
                    galaxy_values.insert(name.as_str().into(), galaxy);
                    if !note.is_empty() {
                        galaxy_notes.push(note);
                    }
                }
            }
        }
        if !galaxy_values.is_empty() {
            step.extra.insert("galaxy_values".into(), Value::Mapping(galaxy_values));
        }
        if !galaxy_notes.is_empty() {
            step.extra
                .insert("galaxy_note".into(), Value::String(galaxy_notes.join(" ")));
        }

        // wiring
        for (gname, up, up_out) in &g.conns {
            let Some(input) = gmap.inputs.get(gname) else {
                notes
                    .entry(g.n)
                    .or_default()
                    .push(format!("Galaxy input '{gname}' is not used here."));
                continue;
            };
            let Some(spec) = tool.inputs.get(input) else { continue };
            let source = match kinds.get(up) {
                Some(Kind::Input) if spec.kind == "reads" => Some(INPUT.to_string()),
                Some(Kind::Tool { tool: up_tool, .. }) => gmaps
                    .get(up_tool)
                    .and_then(|m| m.outputs.get(up_out))
                    .filter(|o| all[up_tool].outputs.contains_key(*o))
                    .map(|o| format!("{}.{o}", step_ids[up])),
                _ => None,
            };
            let Some(source) = source else {
                notes.entry(g.n).or_default().push(format!(
                    "Its input '{input}' came from Galaxy step {up}, which is not imported."
                ));
                continue;
            };
            let mut sources = vec![source.clone()];
            // when the step before only cleans the same kind of data (fastp), fall
            // back on what it read, so switching it off still leaves this step fed
            if let Some((up_id, up_out)) = source.split_once('.') {
                if let Some(up_step) = steps.get(up_id) {
                    let up_tool = &all[&up_step.tool];
                    let kind = up_tool.outputs.get(up_out).map(|o| o.kind.as_str());
                    for (iname, ispec) in &up_tool.inputs {
                        if Some(ispec.kind.as_str()) == kind {
                            sources.extend(up_step.inputs.get(iname).cloned().unwrap_or_default());
                        }
                    }
                }
            }
            let list = step.inputs.entry(input.clone()).or_default();
            for s in sources {
                if !list.contains(&s) {
                    list.push(s);
                }
            }
        }
        steps.insert(sid, step);
    }

    // ---- a step whose required input has nothing to read is left out (and so on down)
    let mut dropped: HashMap<String, String> = HashMap::new();
    loop {
        let bad: Vec<(String, String)> = steps
            .iter()
            .filter_map(|(sid, st)| {
                let missing = all[&st.tool]
                    .inputs
                    .iter()
                    .find(|(n, spec)| !spec.optional && st.inputs.get(*n).is_none_or(Vec::is_empty))?;
                Some((sid.clone(), missing.0.clone()))
            })
            .collect();
        if bad.is_empty() {
            break;
        }
        for (sid, input) in bad {
            steps.shift_remove(&sid);
            dropped.insert(
                sid.clone(),
                format!("Left out: its input '{input}' comes from a step that is not imported."),
            );
            for st in steps.values_mut() {
                for list in st.inputs.values_mut() {
                    list.retain(|s| s.split('.').next() != Some(sid.as_str()));
                }
                st.inputs.retain(|_, l| !l.is_empty());
            }
        }
    }

    // ---- Galaxy's text steps: one summary step where the app's summary tool covers them
    let utility: BTreeSet<u32> = gsteps
        .iter()
        .filter(|g| matches!(kinds.get(&g.n), Some(Kind::Utility)))
        .map(|g| g.n)
        .collect();
    // (tool step, its Galaxy output) read by a text step
    let fed: HashSet<(u32, &str)> = gsteps
        .iter()
        .filter(|g| utility.contains(&g.n))
        .flat_map(|g| g.conns.iter())
        .filter(|(_, up, _)| step_ids.get(up).is_some_and(|s| steps.contains_key(s)))
        .map(|(_, up, out)| (*up, out.as_str()))
        .collect();
    let mut covered: BTreeSet<u32> = BTreeSet::new();
    let mut summary_id: Option<String> = None;
    if !utility.is_empty() {
        for (tool_id, gmap) in gmaps.iter().filter(|(_, m)| !m.sources.is_empty()) {
            let tool = &all[tool_id];
            let mut step = Step {
                tool: tool_id.clone(),
                label: Some(tool.name.clone()),
                ..Step::default()
            };
            let mut wired: HashSet<(u32, &str)> = HashSet::new();
            for (input, spec) in &gmap.sources {
                let hit = gsteps.iter().find_map(|g| {
                    let sid = step_ids.get(&g.n)?;
                    let st = steps.get(sid)?;
                    if st.tool != spec.tool {
                        return None;
                    }
                    let up_tool = &all[&st.tool];
                    let same = spec.settings.iter().all(|(k, want)| {
                        let have = st
                            .settings
                            .get(k)
                            .or_else(|| up_tool.settings.get(k).map(|s| &s.default));
                        have.map(to_text) == Some(to_text(want))
                    });
                    let up_map = gmaps.get(&st.tool)?;
                    let edge = fed
                        .iter()
                        .find(|(n, o)| *n == g.n && up_map.outputs.get(*o) == Some(&spec.output))
                        .copied();
                    (same && (edge.is_some() || spec.always)).then(|| (edge, format!("{sid}.{}", spec.output)))
                });
                if let Some((edge, source)) = hit {
                    wired.extend(edge);
                    step.inputs.insert(input.clone(), vec![source]);
                }
            }
            if step.inputs.is_empty() {
                continue;
            }
            // text steps hang together in chains: a chain is covered when it reads a wired output
            let mut group: HashMap<u32, u32> = utility.iter().map(|n| (*n, *n)).collect();
            fn root(group: &HashMap<u32, u32>, mut n: u32) -> u32 {
                while group[&n] != n {
                    n = group[&n];
                }
                n
            }
            for n in &utility {
                for (_, up, _) in &by_n[n].conns {
                    if utility.contains(up) {
                        let (a, b) = (root(&group, *n), root(&group, *up));
                        group.insert(a, b);
                    }
                }
            }
            let good: HashSet<u32> = utility
                .iter()
                .filter(|n| {
                    by_n[*n]
                        .conns
                        .iter()
                        .any(|(_, up, out)| wired.contains(&(*up, out.as_str())))
                })
                .map(|n| root(&group, *n))
                .collect();
            covered = utility
                .iter()
                .copied()
                .filter(|n| good.contains(&root(&group, *n)))
                .collect();
            step.extra.insert("galaxy_step".into(), Value::String(ranges(&covered)));
            for (k, v) in &gmap.step {
                match k.as_str() {
                    Some("publish") => step.publish = v.as_str().map(str::to_string),
                    _ => {
                        step.extra.insert(k.clone(), v.clone());
                    }
                }
            }
            let sid = unique(tool_id, &|c| steps.contains_key(c));
            steps.insert(sid.clone(), step);
            summary_id = Some(sid);
            break;
        }
    }

    // ---- the report, one line per Galaxy step
    let mut report = Vec::new();
    for g in &gsteps {
        let mut row = StepReport {
            galaxy_step: g.n,
            name: g.label.clone().unwrap_or_else(|| {
                if g.name.is_empty() {
                    g.key.clone()
                } else {
                    g.name.clone()
                }
            }),
            galaxy_tool: g.key.clone(),
            galaxy_version: g.version.clone(),
            action: "left_out".into(),
            step: None,
            tool: None,
            tool_version: None,
            note: String::new(),
        };
        let mut extra = notes.remove(&g.n).unwrap_or_default();
        match &kinds[&g.n] {
            Kind::Input => {
                row.action = "input".into();
                row.note = "The reads you pick when you start a run.".into();
            }
            Kind::LeftOut(why) => row.note = why.clone(),
            Kind::Utility if covered.contains(&g.n) => {
                row.action = "summary".into();
                row.step = summary_id.clone();
                row.tool = summary_id.as_ref().map(|s| steps[s].tool.clone());
                row.note = "Galaxy text step: covered by the summary tables step.".into();
            }
            Kind::Utility => {
                row.note =
                    "Galaxy text step with no match here. The files of the tools before it are still in the results."
                        .into();
            }
            Kind::Tool { tool, .. } => {
                let sid = &step_ids[&g.n];
                let t = &all[tool];
                row.tool = Some(tool.clone());
                if let Some(why) = dropped.get(sid) {
                    row.note = why.clone();
                } else {
                    let (action, version) = actions[&g.n].clone();
                    row.action = action.into();
                    row.step = Some(sid.clone());
                    row.tool_version = Some(version.unwrap_or_else(|| t.version.clone()));
                    let want = tool_version(&g.version);
                    row.note = match action {
                        "builtin" if want == t.version || t.variants.contains_key(&want) => {
                            format!("{} {want}: already in the app.", t.name)
                        }
                        "builtin" => format!("{} {}: the app's own tool stands in for it.", t.name, t.version),
                        "new_version" => format!(
                            "{} {want} is added beside the app's {} (from BioContainers).",
                            t.name, t.version
                        ),
                        _ => "New tool: its container, inputs, outputs and settings are set up. \
                              Its command still has to be written before it can run."
                            .into(),
                    };
                }
            }
        }
        if !extra.is_empty() && row.action != "left_out" {
            extra.insert(0, std::mem::take(&mut row.note));
            row.note = extra.join(" ");
        }
        report.push(row);
    }
    // versions and drafts of steps that were left out are not needed
    new_versions.retain(|v| {
        steps
            .values()
            .any(|s| s.tool == v.tool && s.tool_version.as_deref() == Some(&v.version))
    });
    drafts.retain(|id, _| steps.values().any(|s| s.tool == *id));

    if !steps.values().any(|s| !all[&s.tool].inputs.is_empty()) || steps.is_empty() {
        bail!("nothing in this Galaxy workflow matches a tool this app can run");
    }

    let name = ga
        .get("name")
        .and_then(Json::as_str)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or("Imported Galaxy workflow")
        .trim()
        .to_string();
    let mut extra = Mapping::new();
    extra.insert("source".into(), Value::String(format!("Galaxy workflow: {name}")));
    let mut input = Mapping::new();
    input.insert("type".into(), "paired_reads".into());
    input.insert("pattern".into(), "*_R{1,2}*.gz".into());
    extra.insert("input".into(), Value::Mapping(input));
    let workflow = Workflow {
        id: unique(&slug(&name), taken),
        name,
        version: 1,
        description: ga
            .get("annotation")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        steps,
        extra,
        saved_version: 0,
        builtin: false,
        file: Default::default(),
    };
    Ok(Import {
        workflow,
        steps: report,
        new_versions,
        new_tools: drafts.into_values().collect(),
    })
}

// ---------------------------------------------------------------- apply

/// What the Tool Shed says about a tool (looked up by [`Lookup::shed_tool`]).
#[derive(Debug, Clone, Default)]
pub struct ShedInfo {
    pub name: String,
    pub description: String,
    pub homepage: String,
    pub category: String,
    /// (package, version) the tool needs, first = the tool itself
    pub requirements: Vec<(String, String)>,
}

/// The look-ups an import needs from the internet (the server does them; tests fake them).
pub trait Lookup {
    /// Newest BioContainers image of this package at this version, e.g.
    /// `quay.io/biocontainers/shovill:1.4.2--hdfd78af_1`.
    fn biocontainer(&self, package: &str, version: &str) -> Result<Option<String>>;
    fn shed_tool(&self, shed: &ShedRef) -> Result<Option<ShedInfo>>;
}

fn safe(v: &str) -> String {
    v.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._+-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dest)?;
        } else {
            fs::copy(e.path(), dest)?;
        }
    }
    Ok(())
}

/// Add another version of a tool beside its default one: the same tool
/// definition with another container, in `<home>/tool_versions/<id>/<version>/`.
/// It is "untested" until a run with it succeeds (as in v1).
pub fn add_tool_version(cat: &Catalog, base: &Tool, version: &str, container: &str) -> Result<()> {
    let dir = cat.home.join("tool_versions").join(&base.id).join(safe(version));
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&base.dir, &dir)?;
    let Value::Mapping(mut raw) = load_yaml(&base.dir.join("tool.yml"))? else {
        bail!("{}: not a mapping", base.dir.join("tool.yml").display())
    };
    raw.insert("version".into(), Value::String(version.into()));
    raw.insert("container".into(), Value::String(container.into()));
    raw.insert("untested".into(), Value::Bool(true));
    raw.insert("based_on".into(), Value::String(base.version.clone()));
    dump_yaml(&raw, &dir.join("tool.yml"))
}

/// The BioContainers package a quay.io image is of: `quay.io/biocontainers/shovill:1.4.2--x` -> `shovill`.
pub fn biocontainers_package(container: &str) -> Option<&str> {
    container.strip_prefix(QUAY)?.split(':').next()
}

#[derive(Debug, Clone, Serialize)]
pub struct Applied {
    pub id: String,
    pub version: u32,
    pub steps: Vec<StepReport>,
}

/// Add the tool versions and tools the import needs, then save the workflow.
pub fn apply(cat: &Catalog, mut imp: Import, look: &dyn Lookup) -> Result<Applied> {
    let tools = cat.load_tools()?;
    for nv in &imp.new_versions {
        let base = tools.get(&nv.tool).ok_or_else(|| anyhow!("unknown tool {}", nv.tool))?;
        let image = match biocontainers_package(&base.container) {
            Some(package) => look.biocontainer(package, &nv.version),
            None => Ok(None),
        };
        let problem = match image {
            Ok(Some(image)) => match add_tool_version(cat, base, &nv.version, &image) {
                Ok(()) => continue,
                Err(e) => format!("{e:#}"),
            },
            Ok(None) => "BioContainers has no container of that version".to_string(),
            Err(e) => format!("the look-up failed ({e:#})"),
        };
        // carry on with the version the app has, and say so
        for (sid, st) in imp.workflow.steps.iter_mut() {
            if st.tool == nv.tool && st.tool_version.as_deref() == Some(&nv.version) {
                st.tool_version = None;
                for row in imp.steps.iter_mut().filter(|r| r.step.as_deref() == Some(sid)) {
                    row.action = "builtin".into();
                    row.tool_version = Some(base.version.clone());
                    row.note = format!(
                        "{} {} could not be added: {problem}. The app's {} is used instead.",
                        base.name, nv.version, base.version
                    );
                }
            }
        }
    }
    for draft in &imp.new_tools {
        let dir = cat.home.join("tools").join(IMPORT_SOURCE).join(&draft.id);
        if dir.join("tool.yml").is_file() {
            continue; // from an earlier import: keep it, the command may be written by now
        }
        let mut yaml = draft.yaml.clone();
        let mut problem = "No container was found for it.".to_string();
        if let Some(Ok(Some(info))) = draft.shed.as_ref().map(|s| look.shed_tool(s)) {
            for (k, v) in [
                ("name", &info.name),
                ("homepage", &info.homepage),
                ("category", &info.category),
            ] {
                if !v.is_empty() {
                    yaml.insert(k.into(), Value::String(v.clone()));
                }
            }
            if !info.description.is_empty() {
                yaml.insert(
                    "description".into(),
                    Value::String(format!(
                        "{} (Imported from Galaxy: its command still has to be written.)",
                        info.description.trim_end_matches('.')
                    )),
                );
            }
            if let Some((package, version)) = info.requirements.first() {
                if let Ok(Some(image)) = look.biocontainer(package, version) {
                    yaml.insert("container".into(), Value::String(image));
                    problem.clear();
                }
            }
        }
        if !problem.is_empty() {
            for row in imp.steps.iter_mut().filter(|r| r.tool.as_deref() == Some(&draft.id)) {
                row.note = format!("{} {problem}", row.note);
            }
        }
        dump_yaml(&yaml, &dir.join("tool.yml"))?;
    }
    let tools = cat.load_tools()?;
    compiler::resolve(&imp.workflow, &tools)
        .map_err(|e| anyhow!("the imported workflow does not fit together: {e}"))?;
    if !ID_RE.is_match(&imp.workflow.id) {
        bail!("workflow id: lowercase letters, digits and _ only, starting with a letter");
    }
    let version = cat.save_workflow(&imp.workflow, None)?;
    Ok(Applied {
        id: imp.workflow.id,
        version,
        steps: imp.steps,
    })
}
