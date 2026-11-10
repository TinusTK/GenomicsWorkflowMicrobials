//! Tools the user adds and edits on this computer (nothing here changes a
//! repository on GitHub): add a tool from a GitHub link, edit its definition,
//! show its help, try it on one sample, add other versions, and the checks for
//! newer tool versions and a newer app.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use gwm_core::catalog::{dump_yaml, load_yaml, ID_RE};
use gwm_core::galaxy::{add_tool_version, biocontainers_package};
use gwm_core::reads::find_samples;
use gwm_core::{paths, Catalog, Tool};
use gwm_engine::trial::{self, Trial};
use gwm_engine::Limits;
use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use serde_yaml::{Mapping, Value};

use crate::import::{biocontainer_versions, version_key};
use crate::jobs::Reporter;

/// The folder under the data folder's tools/ that tools added from GitHub go in.
pub const GITHUB_SOURCE: &str = "github";
pub(crate) const AGENT: &str = "GenomicsWorkflowMicrobials";
const APP_REPO: &str = "TinusTK/GenomicsWorkflowMicrobials";

/// Names used in a command as `$IN_<name>` and `$S_<name>`.
static NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_]*$").unwrap());

/// The text at a web address; None when there is nothing there.
fn fetch(url: &str) -> Result<Option<String>> {
    match ureq::get(url)
        .set("User-Agent", AGENT)
        .timeout(Duration::from_secs(30))
        .call()
    {
        Ok(r) => Ok(Some(r.into_string()?)),
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(ureq::Error::Status(403 | 429, _)) if url.contains("api.github.com") => {
            bail!("GitHub is not answering right now (too many requests from this network). Try again in an hour.")
        }
        Err(e) => Err(anyhow!(e)).with_context(|| format!("asking {url}")),
    }
}

pub(crate) fn fetch_json(url: &str) -> Result<Option<serde_json::Value>> {
    Ok(match fetch(url)? {
        Some(t) => Some(serde_json::from_str(&t)?),
        None => None,
    })
}

pub(crate) fn y(s: impl Into<String>) -> Value {
    Value::String(s.into())
}

// ---------------------------------------------------------------- add from GitHub

/// `https://github.com/owner/repo`, with or without the address in front.
fn parse_repo(link: &str) -> Result<(String, String)> {
    let s = link.trim().trim_end_matches('/');
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let s = s.strip_prefix("www.").unwrap_or(s);
    let s = s.strip_prefix("github.com/").unwrap_or(s);
    let mut parts = s.split('/');
    let ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    match (parts.next(), parts.next().map(|r| r.trim_end_matches(".git"))) {
        (Some(owner), Some(repo)) if ok(owner) && ok(repo) && !owner.contains('.') => {
            Ok((owner.to_string(), repo.to_string()))
        }
        _ => bail!("paste the tool's GitHub link, like https://github.com/owner/tool"),
    }
}

/// BioContainers package names a repository's README points at, most likely first.
fn packages_named(readme: &str) -> Vec<String> {
    static LINKS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:bioconda\.github\.io/recipes/|anaconda\.org/bioconda/|quay\.io/biocontainers/|bioconda::)([a-z0-9][a-z0-9_.-]*)")
            .unwrap()
    });
    let mut out: Vec<String> = LINKS
        .captures_iter(readme)
        .map(|c| c[1].trim_end_matches(['.', '-']).to_lowercase())
        .collect();
    // `conda install -c bioconda tb-profiler`: the last word that is not an option
    for line in readme.lines().filter(|l| {
        let l = l.to_lowercase();
        (l.contains("conda install") || l.contains("mamba install")) && l.contains("bioconda")
    }) {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(at) = words.iter().position(|w| *w == "install") else {
            continue;
        };
        let mut skip = false;
        for w in &words[at + 1..] {
            if skip {
                skip = false;
            } else if *w == "-c" || *w == "--channel" || *w == "-n" || *w == "--name" {
                skip = true;
            } else if !w.starts_with('-') {
                let name = w.split(['=', '<', '>']).next().unwrap_or(w).trim_matches('`');
                out.push(name.to_lowercase());
            }
        }
    }
    let mut seen = Vec::new();
    for p in out {
        if p.len() > 1 && p.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) && !seen.contains(&p) {
            seen.push(p);
        }
    }
    seen
}

fn tool_id(name: &str) -> String {
    let mut id: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while id.contains("__") {
        id = id.replace("__", "_");
    }
    let id = id.trim_matches('_').to_string();
    if id.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        id
    } else {
        format!("tool_{id}")
    }
}

/// The files a ready-made definition lists, out of the repository's `gwm/` folder.
fn fetch_files(owner: &str, repo: &str, branch: &str, files: &[String], dest: &Path) -> Result<()> {
    let url = format!("https://codeload.github.com/{owner}/{repo}/tar.gz/refs/heads/{branch}");
    let resp = ureq::get(&url)
        .set("User-Agent", AGENT)
        .timeout(Duration::from_secs(600))
        .call()
        .with_context(|| format!("downloading {url}"))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(resp.into_reader()));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        // <repo>-<branch>/gwm/<file>
        let inside: PathBuf = path.components().skip(1).collect();
        let Ok(rel) = inside.strip_prefix("gwm") else { continue };
        if !files.iter().any(|f| rel.starts_with(f)) || !entry.header().entry_type().is_file() {
            continue;
        }
        // never outside the tool's folder
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            continue;
        }
        let to = dest.join(rel);
        if let Some(p) = to.parent() {
            fs::create_dir_all(p)?;
        }
        entry.unpack(&to)?;
    }
    Ok(())
}

/// `recipes/<owner>__<repo>.yml`: the tested setup of a well-known tool.
pub fn recipe_file(full: &str) -> String {
    format!("{}.yml", full.to_lowercase().replace('/', "__"))
}

/// The tested setup for a repository, when the app has one: the newest from
/// the app's own repository (so new ones arrive without an update), else the
/// one that came with the app.
fn recipe(cat: &Catalog, full: &str) -> Option<String> {
    let name = recipe_file(full);
    fetch(&format!(
        "https://raw.githubusercontent.com/{APP_REPO}/main/recipes/{name}"
    ))
    .ok()
    .flatten()
    .or_else(|| fs::read_to_string(cat.root.join("recipes").join(&name)).ok())
}

/// Add a tool from its GitHub repository. A tool with a tested setup
/// (`recipes/`) is ready at once. A repository that carries a ready
/// definition (`gwm/tool.yml`) is installed as it is; any other becomes a draft
/// with what could be found out (name, description, licence, newest release,
/// its BioContainers container), to finish in the tool editor.
pub fn add_from_github(cat: &Catalog, link: &str) -> Result<serde_json::Value> {
    let (owner, repo) = parse_repo(link)?;
    let info = fetch_json(&format!("https://api.github.com/repos/{owner}/{repo}"))?
        .ok_or_else(|| anyhow!("GitHub has no repository {owner}/{repo} (or it is private)"))?;
    let text = |k: &str| info[k].as_str().unwrap_or_default().trim().to_string();
    let (owner, repo) = (
        info["owner"]["login"].as_str().unwrap_or(&owner).to_string(),
        info["name"].as_str().unwrap_or(&repo).to_string(),
    );
    let branch = Some(text("default_branch"))
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "main".into());
    let full = format!("{owner}/{repo}");
    let tools = cat.load_tools()?;
    if let Some(t) = tools.values().find(|t| {
        t.extra
            .get("github")
            .and_then(|g| g.get("repo"))
            .and_then(Value::as_str)
            == Some(full.as_str())
    }) {
        bail!("{} is already here: find it on the Tools page", t.name);
    }
    let raw = format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}");
    let mut github = Mapping::new();
    github.insert(y("repo"), y(&full));
    github.insert(y("branch"), y(&branch));

    // a tool this app has a tested setup for
    if let Some(recipe) = recipe(cat, &full) {
        let Value::Mapping(mut yaml) = serde_yaml::from_str(&recipe).context("reading the tool's tested setup")? else {
            bail!("the tested setup for {full} is not a tool definition");
        };
        let tool: Tool =
            serde_yaml::from_value(Value::Mapping(yaml.clone())).context("reading the tool's tested setup")?;
        if !ID_RE.is_match(&tool.id) || tools.contains_key(&tool.id) {
            bail!("a tool called {} is already here", tool.id);
        }
        yaml.insert(y("github"), Value::Mapping(github));
        yaml.insert(y("recipe"), Value::Bool(true));
        dump_yaml(
            &yaml,
            &cat.home
                .join("tools")
                .join(GITHUB_SOURCE)
                .join(&tool.id)
                .join("tool.yml"),
        )?;
        return Ok(json!({
            "id": tool.id, "name": tool.name, "draft": false,
            "notes": [format!("{} {} has a tested setup: it is ready to use in workflows.", tool.name, tool.version)],
        }));
    }

    // a repository made for this app
    if let Some(ready) = fetch(&format!("{raw}/gwm/tool.yml"))? {
        let Value::Mapping(mut yaml) = serde_yaml::from_str(&ready).context("reading the repository's gwm/tool.yml")?
        else {
            bail!("the repository's gwm/tool.yml is not a tool definition");
        };
        let tool: Tool =
            serde_yaml::from_value(Value::Mapping(yaml.clone())).context("reading the repository's gwm/tool.yml")?;
        if !ID_RE.is_match(&tool.id) {
            bail!("the repository's tool id '{}' is not valid", tool.id);
        }
        if tools.contains_key(&tool.id) {
            bail!("a tool called {} is already here", tool.id);
        }
        let dir = cat.home.join("tools").join(GITHUB_SOURCE).join(&tool.id);
        if !tool.files.is_empty() {
            fetch_files(&owner, &repo, &branch, &tool.files, &dir)?;
        }
        yaml.insert(y("github"), Value::Mapping(github));
        dump_yaml(&yaml, &dir.join("tool.yml"))?;
        return Ok(json!({
            "id": tool.id, "name": tool.name, "draft": false,
            "notes": [format!("{full} carries its own definition for this app: {} {} is ready to use.", tool.name, tool.version)],
        }));
    }

    let mut notes: Vec<String> = Vec::new();
    let readme = ["README.md", "readme.md", "README.rst", "README"]
        .iter()
        .find_map(|f| fetch(&format!("{raw}/{f}")).ok().flatten())
        .unwrap_or_default();
    let lower = repo.to_lowercase();
    let mut candidates = packages_named(&readme);
    for c in [lower.clone(), lower.replace('_', "-"), lower.replace(['_', '-'], "")] {
        if !candidates.contains(&c) {
            candidates.push(c);
        }
    }
    let found = candidates.iter().find_map(|c| {
        biocontainer_versions(c)
            .ok()
            .filter(|v| !v.is_empty())
            .map(|v| (c.clone(), v))
    });
    let release = fetch_json(&format!("https://api.github.com/repos/{full}/releases/latest"))
        .ok()
        .flatten()
        .and_then(|r| {
            r["tag_name"]
                .as_str()
                .map(|t| t.trim_start_matches(['v', 'V']).to_string())
        });
    let mut buildable: Option<(&str, String)> = None;
    let (program, version, container) = match &found {
        Some((package, versions)) => {
            let (version, image) = release
                .as_ref()
                .and_then(|r| versions.iter().find(|(v, _)| v == r))
                .unwrap_or(&versions[0])
                .clone();
            notes.push(format!(
                "Found its container on BioContainers: {package} {version}{}.",
                if release.as_deref() == Some(version.as_str()) {
                    " (the newest release)"
                } else {
                    ""
                }
            ));
            (package.clone(), version, image)
        }
        None => {
            // no ready-made container: one can often be built from the repository
            buildable = crate::build::detect(&full, &branch);
            notes.push(match &buildable {
                Some((kind, _)) => format!(
                    "No ready-made container was found, so the app will build one from {}.",
                    crate::build::source_words(kind)
                ),
                None => "No container was found, and the repository has nothing to build one from: type the tool's container address in the editor (More options).".into(),
            });
            (lower.clone(), release.clone().unwrap_or_default(), String::new())
        }
    };
    let mut id = tool_id(&program);
    while tools.contains_key(&id) {
        id.push_str("_2");
    }

    let mut m = Mapping::new();
    m.insert(y("id"), y(&id));
    m.insert(y("name"), y(&repo));
    m.insert(y("version"), y(&version));
    m.insert(y("category"), y("Added from GitHub"));
    m.insert(y("description"), y(text("description")));
    m.insert(
        y("homepage"),
        y(Some(text("homepage"))
            .filter(|h| h.starts_with("http"))
            .unwrap_or_else(|| format!("https://github.com/{full}"))),
    );
    if let Some(l) = info["license"]["spdx_id"].as_str().filter(|l| *l != "NOASSERTION") {
        m.insert(y("license"), y(l));
    }
    m.insert(y("container"), y(&container));
    m.insert(y("scope"), y("per_sample"));
    m.insert(y("resources"), serde_yaml::from_str("{cpus: 4, memory_gb: 8}").unwrap());
    m.insert(
        y("inputs"),
        serde_yaml::from_str("{reads: {type: reads, description: The paired reads of one sample.}}").unwrap(),
    );
    m.insert(y("outputs"), Value::Mapping(Mapping::new()));
    m.insert(y("settings"), Value::Mapping(Mapping::new()));
    m.insert(y("version_command"), y(format!("{program} --version")));
    m.insert(y("command"), y(""));
    m.insert(y("draft"), Value::Bool(true));
    m.insert(y("program"), y(&program));
    m.insert(y("github"), Value::Mapping(github));
    if let Some((kind, file)) = &buildable {
        let mut b = Mapping::new();
        b.insert(y("kind"), y(*kind));
        b.insert(y("file"), y(file));
        m.insert(y("build"), Value::Mapping(b));
    }
    dump_yaml(
        &m,
        &cat.home.join("tools").join(GITHUB_SOURCE).join(&id).join("tool.yml"),
    )?;
    notes.push("Press Set it up for me and the app works out the rest.".into());
    Ok(json!({"id": id, "name": repo, "draft": true, "notes": notes}))
}

// ---------------------------------------------------------------- edit

#[derive(Deserialize)]
pub struct InputDef {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize)]
pub struct OutputDef {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize)]
pub struct SettingDef {
    #[serde(default)]
    label: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    default: serde_json::Value,
    #[serde(default)]
    description: String,
    #[serde(default)]
    options: Vec<String>,
}

/// A tool as the editor sends it.
#[derive(Deserialize)]
pub struct Definition {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    container: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    program: String,
    #[serde(default)]
    version_command: String,
    #[serde(default)]
    command: String,
    cpus: Option<u32>,
    memory_gb: Option<u32>,
    #[serde(default)]
    inputs: IndexMap<String, InputDef>,
    #[serde(default)]
    outputs: IndexMap<String, OutputDef>,
    #[serde(default)]
    settings: IndexMap<String, SettingDef>,
    /// false: a draft, workflows refuse to run it
    #[serde(default)]
    ready: bool,
}

/// A tool of this computer (not one that ships with the app).
pub(crate) fn own_tool(cat: &Catalog, id: &str) -> Result<Tool> {
    let tool = cat
        .load_tools()?
        .swap_remove(id)
        .ok_or_else(|| anyhow!("unknown tool {id}"))?;
    if tool.source == "built-in" || !tool.dir.starts_with(&cat.home) {
        bail!("{} is built in and can't be changed", tool.name);
    }
    Ok(tool)
}

/// The entry called `name`, kept from before (it may hold more than the editor shows).
fn entry(old: Option<&Value>, name: &str) -> Mapping {
    old.and_then(|o| o.get(name))
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default()
}

fn put(m: &mut Mapping, key: &str, text: &str) {
    if text.trim().is_empty() {
        m.remove(key);
    } else {
        m.insert(y(key), y(text.trim()));
    }
}

/// Save the editor's changes into the tool's own tool.yml on this computer.
pub fn save_definition(cat: &Catalog, id: &str, d: Definition) -> Result<()> {
    let tool = own_tool(cat, id)?;
    let file = tool.dir.join("tool.yml");
    let Value::Mapping(mut raw) = load_yaml(&file)? else {
        bail!("{}: not a tool definition", file.display())
    };
    if d.name.trim().is_empty() {
        bail!("give the tool a name");
    }
    for (what, names) in [
        ("input", d.inputs.keys().collect::<Vec<_>>()),
        ("output", d.outputs.keys().collect()),
        ("setting", d.settings.keys().collect()),
    ] {
        if let Some(bad) = names.iter().find(|n| !NAME_RE.is_match(n)) {
            bail!("{what} name '{bad}': letters, digits and _ only, starting with a letter");
        }
    }
    for (name, o) in &d.outputs {
        let p = o.path.trim();
        if p.is_empty() {
            bail!("output {name}: say where its file is, for example out/{{sample}}.{name}.tsv");
        }
        if p.starts_with('/') || p.split('/').any(|part| part == "..") {
            bail!("output {name}: the path stays inside the step's folder (start it with out/)");
        }
    }
    if d.ready {
        if d.command.trim().is_empty() {
            bail!("write the command before switching on Ready to use");
        }
        if d.container.trim().is_empty() {
            bail!("fill in the container before switching on Ready to use");
        }
    }

    raw.insert(y("name"), y(d.name.trim()));
    raw.insert(y("version"), y(d.version.trim()));
    put(&mut raw, "category", &d.category);
    put(&mut raw, "description", &d.description);
    put(&mut raw, "homepage", &d.homepage);
    raw.insert(y("container"), y(d.container.trim()));
    raw.insert(
        y("scope"),
        y(if d.scope == "all_samples" {
            "all_samples"
        } else {
            "per_sample"
        }),
    );
    let mut resources = entry(Some(&Value::Mapping(raw.clone())), "resources");
    resources.insert(y("cpus"), Value::Number(d.cpus.unwrap_or(1).max(1).into()));
    resources.insert(y("memory_gb"), Value::Number(d.memory_gb.unwrap_or(2).max(1).into()));
    raw.insert(y("resources"), Value::Mapping(resources));

    let old = raw.get("inputs").cloned();
    let mut inputs = Mapping::new();
    for (name, i) in &d.inputs {
        let mut e = entry(old.as_ref(), name);
        e.insert(y("type"), y(if i.kind.is_empty() { "any" } else { &i.kind }));
        if i.optional {
            e.insert(y("optional"), Value::Bool(true));
        } else {
            e.remove("optional");
        }
        put(&mut e, "description", &i.description);
        inputs.insert(y(name), Value::Mapping(e));
    }
    raw.insert(y("inputs"), Value::Mapping(inputs));

    let old = raw.get("outputs").cloned();
    let mut outputs = Mapping::new();
    for (name, o) in &d.outputs {
        let mut e = entry(old.as_ref(), name);
        e.insert(y("type"), y(if o.kind.is_empty() { "any" } else { &o.kind }));
        e.insert(y("path"), y(o.path.trim()));
        if o.optional {
            e.insert(y("optional"), Value::Bool(true));
        } else {
            e.remove("optional");
        }
        put(&mut e, "description", &o.description);
        outputs.insert(y(name), Value::Mapping(e));
    }
    raw.insert(y("outputs"), Value::Mapping(outputs));

    let old = raw.get("settings").cloned();
    let mut settings = Mapping::new();
    for (name, s) in &d.settings {
        let mut e = entry(old.as_ref(), name);
        e.insert(
            y("label"),
            y(if s.label.trim().is_empty() {
                name.as_str()
            } else {
                s.label.trim()
            }),
        );
        e.insert(y("type"), y(if s.kind.is_empty() { "string" } else { &s.kind }));
        e.insert(y("default"), serde_yaml::to_value(&s.default)?);
        put(&mut e, "description", &s.description);
        if s.kind == "select" {
            e.insert(
                y("options"),
                Value::Sequence(s.options.iter().map(|o| y(o.trim())).collect()),
            );
        } else {
            e.remove("options");
        }
        settings.insert(y(name), Value::Mapping(e));
    }
    raw.insert(y("settings"), Value::Mapping(settings));

    put(&mut raw, "program", &d.program);
    put(&mut raw, "version_command", &d.version_command);
    raw.insert(y("command"), y(d.command.trim_end()));
    if d.ready {
        raw.remove("draft");
    } else {
        raw.insert(y("draft"), Value::Bool(true));
    }
    // it must still read as a tool
    serde_yaml::from_value::<Tool>(Value::Mapping(raw.clone())).context("these changes don't make a valid tool")?;
    dump_yaml(&raw, &file)
}

/// Remove a tool of this computer, with its extra versions. Refused while one
/// of the user's workflows uses it.
pub fn delete_tool(cat: &Catalog, id: &str) -> Result<()> {
    let tool = own_tool(cat, id)?;
    for w in cat.list_workflows()?.into_iter().filter(|w| !w.builtin) {
        if cat.load_workflow(&w.id, None)?.steps.values().any(|s| s.tool == id) {
            bail!(
                "the workflow \"{}\" uses {}: take the step out (or delete the workflow) first",
                w.name,
                tool.name
            );
        }
    }
    fs::remove_dir_all(&tool.dir)?;
    let _ = fs::remove_dir_all(cat.home.join("tool_versions").join(id));
    Ok(())
}

// ---------------------------------------------------------------- help and try

fn trial<'a>(cat: &Catalog, tool: &'a Tool, kind: &str) -> Result<Trial<'a>> {
    let limits = Limits::detect(0, 0);
    Ok(Trial {
        tool,
        command: None,
        sample: "sample".into(),
        inputs: IndexMap::new(),
        dir: cat.home.join("try").join(format!("{}-{kind}", tool.id)),
        db_dir: paths::db_dir(&cat.home),
        databases: cat
            .load_databases()?
            .into_iter()
            .map(|(id, db)| (id, db.install_dir))
            .collect(),
        cpus: tool.resources.cpus.unwrap_or(2).clamp(1, limits.cpus.max(1)),
        memory_gb: tool.resources.memory_gb.unwrap_or(4).clamp(1, limits.memory_gb.max(1)),
    })
}

/// Run `<program> --help` in the tool's container and keep what it prints.
pub fn show_help(cat: &Catalog, id: &str, program: &str, r: &Reporter) -> Result<()> {
    let tool = cat
        .load_tools()?
        .swap_remove(id)
        .ok_or_else(|| anyhow!("unknown tool {id}"))?;
    r.log("Getting the container if it is not here yet (the first time can take a few minutes)");
    let printed = help_text(cat, &tool, program)?;
    if printed.trim().is_empty() {
        bail!("{program} printed nothing: check the program's name and the container");
    }
    r.result(json!({"text": printed}));
    Ok(())
}

/// What `<program> --help` prints in the tool's container (empty when nothing).
pub(crate) fn help_text(cat: &Catalog, tool: &Tool, program: &str) -> Result<String> {
    let program = program.trim();
    if program.is_empty() || program.contains(['\n', ';', '|', '&', '`', '$']) {
        bail!(
            "type the program's name as you would in a terminal, for example {}",
            tool.id
        );
    }
    let mut t = trial(cat, tool, "help")?;
    t.command = Some(format!(
        "{{ {program} --help || {program} -h || {program} help || true; }} 2>&1 | head -n 400"
    ));
    let done = trial::run(&t, gwm_engine::backend::detect().as_ref(), &AtomicBool::new(false))?;
    // what the program printed, without the container download lines
    let printed = fs::read_to_string(done.dir.join(".command.out")).unwrap_or_default();
    let _ = fs::remove_dir_all(&done.dir);
    Ok(printed.trim_end().to_string())
}

#[derive(Deserialize, Default)]
pub struct TryRequest {
    /// a folder of reads: its first sample is used for inputs that take reads
    #[serde(default)]
    pub(crate) reads_folder: String,
    /// input -> a file on this computer, for the other inputs
    #[serde(default)]
    pub(crate) files: IndexMap<String, String>,
}

/// Run the tool's command once on one sample and report what it made.
pub fn try_tool(cat: &Catalog, id: &str, req: TryRequest, r: &Reporter) -> Result<()> {
    let tool = cat
        .load_tools()?
        .swap_remove(id)
        .ok_or_else(|| anyhow!("unknown tool {id}"))?;
    if tool.command.trim().is_empty() {
        bail!("write the command first");
    }
    r.log(format!(
        "Running {} (getting the container first if it is not here yet)",
        tool.name
    ));
    let done = run_try(cat, &tool, &req)?;
    r.result(serde_json::to_value(&done)?);
    if done.exit != 0 {
        bail!("the command stopped with an error (exit code {})", done.exit);
    }
    Ok(())
}

/// The tool's command, once, on the first sample of the reads folder (and the
/// files given for its other inputs).
pub(crate) fn run_try(cat: &Catalog, tool: &Tool, req: &TryRequest) -> Result<trial::TrialResult> {
    let mut t = trial(cat, tool, "sample")?;
    for (name, spec) in &tool.inputs {
        if spec.kind == "reads" {
            if req.reads_folder.trim().is_empty() {
                bail!("choose a folder of reads to try with");
            }
            let found = find_samples(&paths::native_path(req.reads_folder.trim()), None);
            let pairs: Vec<_> = found.samples.iter().filter(|s| s.r2.is_some()).collect();
            let Some(first) = pairs.first() else {
                bail!("no paired reads found in {}", req.reads_folder.trim());
            };
            t.sample = first.sample.clone();
            // an all-sample tool gets two samples, a per-sample tool one
            let take = if tool.per_sample() { 1 } else { 2 };
            t.inputs.insert(
                name.clone(),
                pairs
                    .iter()
                    .take(take)
                    .flat_map(|s| [s.r1.clone(), s.r2.clone().unwrap_or_default()])
                    .collect(),
            );
        } else if let Some(f) = req.files.get(name).map(|f| f.trim()).filter(|f| !f.is_empty()) {
            let path = paths::native_path(f);
            if !path.is_file() {
                bail!("input {name}: no file at {f}");
            }
            t.inputs.insert(name.clone(), vec![path]);
        } else if !spec.optional {
            bail!("input {name}: give a file to try with");
        }
    }
    if !tool.per_sample() {
        t.sample = "all".into();
    }
    trial::run(&t, gwm_engine::backend::detect().as_ref(), &AtomicBool::new(false))
}

// ---------------------------------------------------------------- versions and updates

/// The BioContainers package a tool's own version comes from. Helper tools
/// that only borrow a container (python for a script) have none.
fn versioned_package(tool: &Tool) -> Option<&str> {
    biocontainers_package(&tool.container).filter(|_| tool.container.contains(&format!(":{}--", tool.version)))
}

/// The versions of a tool that can be added (BioContainers tools only).
pub fn versions(cat: &Catalog, id: &str) -> Result<serde_json::Value> {
    let tools = cat.load_tools()?;
    let tool = tools.get(id).ok_or_else(|| anyhow!("unknown tool {id}"))?;
    let Some(package) = versioned_package(tool) else {
        return Ok(json!({"package": null, "current": tool.version, "available": []}));
    };
    let available: Vec<serde_json::Value> = biocontainer_versions(package)?
        .into_iter()
        .filter(|(v, _)| *v != tool.version && !tool.variants.contains_key(v))
        .map(|(version, container)| json!({"version": version, "container": container}))
        .collect();
    Ok(json!({"package": package, "current": tool.version, "available": available}))
}

/// Add another version of a tool beside the one it has.
pub fn add_version(cat: &Catalog, id: &str, version: &str) -> Result<()> {
    let tools = cat.load_tools()?;
    let tool = tools.get(id).ok_or_else(|| anyhow!("unknown tool {id}"))?;
    let package = versioned_package(tool).ok_or_else(|| anyhow!("{} has no other versions to add", tool.name))?;
    let (_, image) = biocontainer_versions(package)?
        .into_iter()
        .find(|(v, _)| v == version)
        .ok_or_else(|| anyhow!("BioContainers has no {package} {version}"))?;
    add_tool_version(cat, tool, version, &image)
}

/// Tools with a newer version on BioContainers than any here.
pub fn newer_versions(cat: &Catalog) -> Result<Vec<serde_json::Value>> {
    let tools = cat.load_tools()?;
    let found: Vec<Option<serde_json::Value>> = std::thread::scope(|scope| {
        let checks: Vec<_> = tools
            .values()
            .map(|tool| {
                scope.spawn(move || {
                    let package = versioned_package(tool)?;
                    let (newest, _) = biocontainer_versions(package).ok()?.into_iter().next()?;
                    let have = std::iter::once(&tool.version)
                        .chain(tool.variants.keys())
                        .map(|v| version_key(v))
                        .max()?;
                    (version_key(&newest) > have)
                        .then(|| json!({"id": tool.id, "name": tool.name, "current": tool.version, "newest": newest}))
                })
            })
            .collect();
        checks.into_iter().map(|c| c.join().ok().flatten()).collect()
    });
    Ok(found.into_iter().flatten().collect())
}

/// Is there a newer release of the app on GitHub?
pub fn app_update() -> Result<serde_json::Value> {
    let current = env!("CARGO_PKG_VERSION");
    let page = format!("https://github.com/{APP_REPO}/releases/latest");
    let latest = fetch_json(&format!("https://api.github.com/repos/{APP_REPO}/releases/latest"))?
        .and_then(|r| r["tag_name"].as_str().map(|t| t.trim_start_matches('v').to_string()));
    let newer = latest.as_ref().is_some_and(|l| version_key(l) > version_key(current));
    Ok(json!({"current": current, "latest": latest, "newer": newer, "url": page}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_links() {
        let want = ("jodyphelan".to_string(), "TBProfiler".to_string());
        assert_eq!(parse_repo("https://github.com/jodyphelan/TBProfiler").unwrap(), want);
        assert_eq!(parse_repo("github.com/jodyphelan/TBProfiler.git/").unwrap(), want);
        assert_eq!(
            parse_repo("https://github.com/jodyphelan/TBProfiler/tree/master").unwrap(),
            want
        );
        assert_eq!(parse_repo("jodyphelan/TBProfiler").unwrap(), want);
        assert!(parse_repo("https://gitlab.com/a/b").is_err());
        assert!(parse_repo("TBProfiler").is_err());
    }

    #[test]
    fn packages_from_a_readme() {
        let readme =
            "[![bioconda](https://img.shields.io/x.svg)](http://bioconda.github.io/recipes/tb-profiler/README.html)\n\
                      ```\nconda install -c bioconda -c conda-forge tb-profiler=6.2\n```\n";
        assert_eq!(packages_named(readme), vec!["tb-profiler"]);
        assert_eq!(packages_named("mamba install -n env bioconda::mlst"), vec!["mlst"]);
    }

    #[test]
    fn ids_and_versions() {
        assert_eq!(tool_id("tb-profiler"), "tb_profiler");
        assert_eq!(tool_id("3d-Tool"), "tool_3d_tool");
        assert!(version_key("1.10.0") > version_key("1.9.3"));
    }
}
