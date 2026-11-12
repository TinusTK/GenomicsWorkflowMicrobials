//! Loads tools, workflows and databases from disk (port of v1 `app/catalog.py`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{anyhow, bail, Context, Result};
use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

use crate::{paths, value};

/// Kept with a tool's switched-off outputs (`tool_outputs.json`): its reports are
/// not turned into tables.
pub const REPORT_TABLES: &str = "__report_tables";
/// The output a tool that writes reports offers them as tables under.
pub const TABLES_OUTPUT: &str = "tables";

pub static ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_]*$").unwrap());

// ---------------------------------------------------------------- tools

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Resources {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_gb: Option<u32>,
    /// Memory comes from this setting (GB; 0 = Auto)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_setting: Option<String>,
    /// Boolean setting that lowers Auto memory to `low_memory_gb`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_memory_setting: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_memory_gb: Option<u32>,
}

impl Resources {
    /// Step resources on top of the tool's.
    pub fn merged(&self, over: &Resources) -> Resources {
        Resources {
            cpus: over.cpus.or(self.cpus),
            memory_gb: over.memory_gb.or(self.memory_gb),
            memory_setting: over.memory_setting.clone().or_else(|| self.memory_setting.clone()),
            low_memory_setting: over
                .low_memory_setting
                .clone()
                .or_else(|| self.low_memory_setting.clone()),
            low_memory_gb: over.low_memory_gb.or(self.low_memory_gb),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputSpec {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub optional: bool,
    /// Takes every connected source (any number), not just the first that runs
    #[serde(default)]
    pub collect: bool,
    /// With `collect`: the output types that fit (e.g. tsv, csv)
    #[serde(default)]
    pub accepts: Vec<String>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSpec {
    #[serde(rename = "type", default)]
    pub kind: String,
    /// Path under the task folder; `{sample}` = the sample id; may be a glob
    pub path: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingSpec {
    #[serde(default)]
    pub label: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub default: Value,
    #[serde(flatten)]
    pub extra: Mapping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub id: String,
    pub name: String,
    #[serde(default, deserialize_with = "value::text")]
    pub version: String,
    #[serde(default)]
    pub container: String,
    #[serde(default = "per_sample")]
    pub scope: String,
    #[serde(default)]
    pub resources: Resources,
    #[serde(default)]
    pub inputs: IndexMap<String, InputSpec>,
    #[serde(default)]
    pub outputs: IndexMap<String, OutputSpec>,
    #[serde(default, deserialize_with = "null_map")]
    pub settings: IndexMap<String, SettingSpec>,
    #[serde(default)]
    pub databases: Vec<String>,
    /// Files and folders from the tool's folder copied into the task (`$TOOL_DIR`)
    #[serde(default)]
    pub files: Vec<String>,
    /// name -> shell command printing that program's version
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub versions: Option<IndexMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_command: Option<String>,
    #[serde(default)]
    pub command: String,
    /// Everything else (description, citation, requirements, ...), kept as is
    #[serde(flatten)]
    pub extra: Mapping,

    /// Folder holding this tool.yml
    #[serde(skip)]
    pub dir: PathBuf,
    /// "built-in", or the folder under the data folder's tools/ (github, galaxy_import)
    #[serde(skip)]
    pub source: String,
    /// Folder name inside a run's tools/ (`<id>` or `<id>@<version>`)
    #[serde(skip)]
    pub bundle: String,
    /// Extra versions added on the Tools page: version -> tool
    #[serde(skip)]
    pub variants: IndexMap<String, Tool>,
    /// Its outputs that are reports (text, JSON) the engine also writes as
    /// tables; empty when it has none or that is switched off for the tool
    #[serde(skip)]
    pub report_outputs: Vec<String>,
}

fn per_sample() -> String {
    "per_sample".into()
}

fn null_map<'de, D, T>(d: D) -> Result<IndexMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<IndexMap<String, T>>::deserialize(d)?.unwrap_or_default())
}

impl Tool {
    pub fn per_sample(&self) -> bool {
        self.scope == "per_sample"
    }

    /// name -> version command (`versions:`, else `version_command:` under the tool id)
    pub fn version_commands(&self) -> IndexMap<String, String> {
        match &self.versions {
            Some(v) if !v.is_empty() => v.clone(),
            _ => IndexMap::from([(
                self.id.clone(),
                self.version_command.clone().unwrap_or_else(|| "echo unknown".into()),
            )]),
        }
    }
}

// ---------------------------------------------------------------- workflows

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Step {
    pub tool: String,
    #[serde(
        default,
        deserialize_with = "value::opt_text",
        skip_serializing_if = "Option::is_none"
    )]
    pub tool_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// input name -> sources, first available wins: `input` or `<step>.<output>`
    #[serde(default, deserialize_with = "null_map", skip_serializing_if = "IndexMap::is_empty")]
    pub inputs: IndexMap<String, Vec<String>>,
    #[serde(default, deserialize_with = "null_map", skip_serializing_if = "IndexMap::is_empty")]
    pub settings: IndexMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Results folder instead of samples/<id>/<step> or combined/<step>
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Resources>,
    #[serde(flatten)]
    pub extra: Mapping,
}

impl Step {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub steps: IndexMap<String, Step>,
    #[serde(flatten)]
    pub extra: Mapping,

    /// The saved version this is (built-ins: their `version`)
    #[serde(skip)]
    pub saved_version: u32,
    #[serde(skip)]
    pub builtin: bool,
    #[serde(skip)]
    pub file: PathBuf,
}

fn one() -> u32 {
    1
}

impl Workflow {
    /// What the workflow starts from: "assemblies" (a folder of FASTA files,
    /// one sample each) or "paired_reads" (`input: type:` in its file).
    pub fn input_type(&self) -> &'static str {
        let kind = self
            .extra
            .get("input")
            .and_then(|i| i.get("type"))
            .and_then(Value::as_str);
        if kind == Some(crate::reads::ASSEMBLIES) {
            crate::reads::ASSEMBLIES
        } else {
            "paired_reads"
        }
    }

    pub fn from_assemblies(&self) -> bool {
        self.input_type() == crate::reads::ASSEMBLIES
    }

    /// Reads pattern used when no samples.csv is given.
    pub fn reads_pattern(&self) -> String {
        self.extra
            .get("input")
            .and_then(|i| i.get("pattern"))
            .and_then(Value::as_str)
            .unwrap_or("*_R{1,2}*.gz")
            .to_string()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub builtin: bool,
    pub versions: Vec<u32>,
    pub latest: u32,
    /// "paired_reads" or "assemblies": what a run of it starts from
    pub input_type: String,
}

// ---------------------------------------------------------------- databases

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Database {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default, deserialize_with = "value::text")]
    pub version: String,
    #[serde(default)]
    pub size_gb: f64,
    pub install_dir: String,
    #[serde(default)]
    pub check_file: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub format: String,
    #[serde(flatten)]
    pub extra: Mapping,
    #[serde(default)]
    pub path: PathBuf,
    #[serde(default)]
    pub installed: bool,
}

// ---------------------------------------------------------------- catalog

/// Where the shipped files and the user's data are.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub root: PathBuf,
    pub home: PathBuf,
    /// where run folders go (Documents, Genomics Workflow Microbials)
    pub runs: PathBuf,
}

impl Default for Catalog {
    fn default() -> Self {
        Catalog {
            root: paths::root(),
            home: paths::home(),
            runs: paths::runs_root(),
        }
    }
}

pub fn load_yaml(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let v: Value = serde_yaml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(if v.is_null() { Value::Mapping(Mapping::new()) } else { v })
}

pub fn dump_yaml<T: Serialize>(data: &T, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_yaml::to_string(data)?)?;
    Ok(())
}

/// Sorted paths matching `<dir>/*/<name>` (depth 1) or `<dir>/*/*/<name>` (depth 2).
fn glob_files(dir: &Path, depth: usize, name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut level = vec![dir.to_path_buf()];
    for _ in 0..depth {
        let mut next = Vec::new();
        for d in &level {
            if let Ok(rd) = fs::read_dir(d) {
                next.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
            }
        }
        level = next;
    }
    for d in level {
        let f = d.join(name);
        if f.is_file() {
            out.push(f);
        }
    }
    out.sort();
    out
}

impl Catalog {
    /// Run folders go in `<home>/runs` (tests); [`Catalog::default`] uses Documents.
    pub fn new(root: impl Into<PathBuf>, home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        Catalog {
            root: root.into(),
            runs: home.join("runs"),
            home,
        }
    }

    // ------------------------------------------------------------ tools

    /// {id: tool}: built-in first (wins on a clash), then installed packs; extra
    /// versions from the Tools page go in `variants`.
    pub fn load_tools(&self) -> Result<IndexMap<String, Tool>> {
        let mut tools: IndexMap<String, Tool> = IndexMap::new();
        let builtin = glob_files(&self.root.join("tools"), 1, "tool.yml");
        let installed = glob_files(&self.home.join("tools"), 2, "tool.yml");
        for (f, is_builtin) in builtin
            .iter()
            .map(|f| (f, true))
            .chain(installed.iter().map(|f| (f, false)))
        {
            let mut t: Tool =
                serde_yaml::from_value(load_yaml(f)?).with_context(|| format!("reading {}", f.display()))?;
            let dir = f.parent().unwrap().to_path_buf();
            t.source = if is_builtin {
                "built-in".into()
            } else {
                dir.parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            t.dir = dir;
            t.bundle = t.id.clone();
            tools.entry(t.id.clone()).or_insert(t);
        }
        for f in glob_files(&self.home.join("tool_versions"), 2, "tool.yml") {
            let mut t: Tool =
                serde_yaml::from_value(load_yaml(&f)?).with_context(|| format!("reading {}", f.display()))?;
            let Some(base) = tools.get_mut(&t.id) else { continue };
            let dir = f.parent().unwrap().to_path_buf();
            t.source = base.source.clone();
            t.bundle = format!("{}@{}", t.id, dir.file_name().unwrap().to_string_lossy());
            t.dir = dir;
            base.variants.insert(t.version.clone(), t);
        }
        self.offer_report_tables(&mut tools);
        Ok(tools)
    }

    /// Tools added on this computer that write a report (a text or JSON file)
    /// also offer it as tables, their `tables` output, unless that is switched
    /// off for the tool on the Tools page (`tool_outputs.json`). The engine
    /// writes the tables at the end of the tool's step. Built-in tools are left
    /// as they are (the RDH workflow stays the same as Galaxy's), unless their
    /// tool.yml says `tables_from_reports: true` (TBProfiler).
    fn offer_report_tables(&self, tools: &mut IndexMap<String, Tool>) {
        let off: IndexMap<String, Vec<String>> = fs::read_to_string(self.home.join("tool_outputs.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        fn offer(t: &mut Tool) {
            t.report_outputs = t
                .outputs
                .iter()
                .filter(|(_, o)| o.kind == "txt" || o.kind == "json")
                .map(|(name, _)| name.clone())
                .collect();
            if !t.report_outputs.is_empty() && !t.outputs.contains_key(TABLES_OUTPUT) {
                t.outputs.insert(
                    TABLES_OUTPUT.into(),
                    OutputSpec {
                        kind: "tsv".into(),
                        path: "out/tables/*.tsv".into(),
                        optional: true,
                        description: "Its reports as tables (made by the app): connect this to Summary tables.".into(),
                    },
                );
            }
        }
        for tool in tools.values_mut() {
            // a built-in tool only when its tool.yml asks for it
            let left_alone = tool.source == "built-in"
                && tool.extra.get("tables_from_reports").and_then(Value::as_bool) != Some(true);
            if left_alone || off.get(&tool.id).is_some_and(|o| o.iter().any(|x| x == REPORT_TABLES)) {
                continue;
            }
            tool.variants.values_mut().for_each(offer);
            offer(tool);
        }
    }

    // ------------------------------------------------------------ databases

    pub fn load_databases(&self) -> Result<IndexMap<String, Database>> {
        let cat = load_yaml(&self.root.join("databases/catalog.yml"))?;
        let raw = cat.get("databases").cloned().unwrap_or(Value::Mapping(Mapping::new()));
        let mut dbs: IndexMap<String, Database> =
            serde_yaml::from_value(raw).context("reading databases/catalog.yml")?;
        let root = paths::db_dir(&self.home);
        for (id, db) in dbs.iter_mut() {
            db.id = id.clone();
            db.path = root.join(&db.install_dir);
            db.installed = db.path.join(&db.check_file).exists();
        }
        Ok(dbs)
    }

    // ------------------------------------------------------------ workflows

    fn builtin_raw(&self) -> Result<IndexMap<String, (Mapping, PathBuf)>> {
        let mut out = IndexMap::new();
        for f in glob_files(&self.root.join("workflows"), 1, "workflow.yml") {
            let Value::Mapping(m) = load_yaml(&f)? else {
                bail!("{}: not a mapping", f.display())
            };
            let id = m
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("{}: no id", f.display()))?
                .to_string();
            out.insert(id, (m, f));
        }
        Ok(out)
    }

    /// `extends: <id>` = the parent's steps, then this workflow's own on top.
    fn expand(raw: &IndexMap<String, (Mapping, PathBuf)>, wf: &Mapping, depth: usize) -> Result<Mapping> {
        let Some(parent_id) = wf.get("extends").and_then(Value::as_str) else {
            return Ok(wf.clone());
        };
        if depth > 10 {
            bail!("workflow extends form a loop at {parent_id}");
        }
        let (p, _) = raw
            .get(parent_id)
            .ok_or_else(|| anyhow!("extends unknown workflow {parent_id}"))?;
        let parent = Self::expand(raw, p, depth + 1)?;
        let mut out = parent.clone();
        for (k, v) in wf {
            if k.as_str() != Some("steps") {
                out.insert(k.clone(), v.clone());
            }
        }
        let mut steps = parent
            .get("steps")
            .and_then(Value::as_mapping)
            .cloned()
            .unwrap_or_default();
        if let Some(own) = wf.get("steps").and_then(Value::as_mapping) {
            for (k, v) in own {
                steps.insert(k.clone(), v.clone());
            }
        }
        out.insert("steps".into(), Value::Mapping(steps));
        Ok(out)
    }

    pub fn builtin_workflows(&self) -> Result<Vec<Workflow>> {
        let raw = self.builtin_raw()?;
        let mut out = Vec::new();
        for (_, (m, f)) in &raw {
            let mut wf: Workflow = serde_yaml::from_value(Value::Mapping(Self::expand(&raw, m, 0)?))
                .with_context(|| format!("reading {}", f.display()))?;
            wf.saved_version = wf.version;
            wf.builtin = true;
            wf.file = f.clone();
            out.push(wf);
        }
        Ok(out)
    }

    fn user_dir(&self) -> PathBuf {
        self.home.join("workflows")
    }

    /// Saved versions of a user workflow, oldest first: (version, file).
    pub fn user_workflow_versions(&self, id: &str) -> Vec<(u32, PathBuf)> {
        let mut out: Vec<(u32, PathBuf)> = fs::read_dir(self.user_dir().join(id))
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        let stem = p.file_stem()?.to_str()?;
                        let n = stem.strip_prefix('v')?.parse().ok()?;
                        (p.extension()? == "yml").then_some((n, p))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    pub fn list_workflows(&self) -> Result<Vec<WorkflowSummary>> {
        let mut out: Vec<WorkflowSummary> = self
            .builtin_workflows()?
            .into_iter()
            .map(|wf| WorkflowSummary {
                input_type: wf.input_type().to_string(),
                id: wf.id,
                name: wf.name,
                description: wf.description,
                builtin: true,
                versions: vec![wf.saved_version],
                latest: wf.saved_version,
            })
            .collect();
        let mut dirs: Vec<PathBuf> = fs::read_dir(self.user_dir())
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        dirs.sort();
        for d in dirs {
            let versions = self.user_workflow_versions(&d.file_name().unwrap().to_string_lossy());
            let Some((latest, f)) = versions.last() else { continue };
            let wf: Workflow =
                serde_yaml::from_value(load_yaml(f)?).with_context(|| format!("reading {}", f.display()))?;
            out.push(WorkflowSummary {
                input_type: wf.input_type().to_string(),
                id: wf.id,
                name: wf.name,
                description: wf.description,
                builtin: false,
                versions: versions.iter().map(|(v, _)| *v).collect(),
                latest: *latest,
            });
        }
        Ok(out)
    }

    pub fn load_workflow(&self, id: &str, version: Option<u32>) -> Result<Workflow> {
        if let Some(wf) = self.builtin_workflows()?.into_iter().find(|w| w.id == id) {
            return Ok(wf);
        }
        let versions = self.user_workflow_versions(id);
        let (v, f) = match version {
            None => versions
                .last()
                .cloned()
                .ok_or_else(|| anyhow!("unknown workflow {id}"))?,
            Some(v) => versions
                .iter()
                .find(|(n, _)| *n == v)
                .cloned()
                .ok_or_else(|| anyhow!("workflow {id} has no version {v}"))?,
        };
        let mut wf: Workflow =
            serde_yaml::from_value(load_yaml(&f)?).with_context(|| format!("reading {}", f.display()))?;
        wf.saved_version = v;
        wf.builtin = false;
        wf.file = f;
        Ok(wf)
    }

    /// Delete a user workflow and every saved version. Built-ins can't be deleted;
    /// past runs keep their own copy of the workflow.
    pub fn delete_workflow(&self, id: &str) -> Result<()> {
        if self.builtin_workflows()?.iter().any(|w| w.id == id) {
            bail!("built-in workflows can't be deleted");
        }
        if !ID_RE.is_match(id) || self.user_workflow_versions(id).is_empty() {
            bail!("unknown workflow {id}");
        }
        fs::remove_dir_all(self.user_dir().join(id))?;
        Ok(())
    }

    /// Save as a new version, never overwriting. Built-ins only under a new id.
    pub fn save_workflow(&self, wf: &Workflow, new_id: Option<&str>) -> Result<u32> {
        let mut wf = wf.clone();
        for k in ["locked", "builtin"] {
            wf.extra.remove(k);
        }
        if let Some(new_id) = new_id {
            if !ID_RE.is_match(new_id) {
                bail!("workflow id: lowercase letters, digits and _ only, starting with a letter");
            }
            if !self.user_workflow_versions(new_id).is_empty() {
                bail!("a workflow called {new_id} already exists: pick another name");
            }
            wf.id = new_id.to_string();
        }
        if self.builtin_workflows()?.iter().any(|b| b.id == wf.id) {
            bail!("built-in workflows are read-only: save it under a new id");
        }
        let version = self
            .user_workflow_versions(&wf.id)
            .last()
            .map(|(v, _)| v + 1)
            .unwrap_or(1);
        wf.version = version;
        dump_yaml(&wf, &self.user_dir().join(&wf.id).join(format!("v{version}.yml")))?;
        Ok(version)
    }
}

/// The tool a workflow step uses: its `tool_version` if it picked one, else the default.
pub fn step_tool<'a>(tools: &'a IndexMap<String, Tool>, step: &Step) -> Option<&'a Tool> {
    let t = tools.get(&step.tool)?;
    match step.tool_version.as_deref() {
        None | Some("") => Some(t),
        Some(v) if v == t.version => Some(t),
        Some(v) => t.variants.get(v),
    }
}
