//! A container for a tool that has none on BioContainers: built on this
//! computer with Docker from what the tool's repository offers, tried in this
//! order: its own Dockerfile, a conda environment file, a Python package.
//! The image stays on this computer (another computer builds its own).

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use gwm_core::catalog::{dump_yaml, load_yaml};
use gwm_core::Catalog;
use serde_json::json;
use serde_yaml::{Mapping, Value};

use crate::jobs::Reporter;
use crate::tooledit::{fetch_json, own_tool, y, AGENT};

/// What a repository can be built from: ("dockerfile" | "conda" | "pip", the file).
pub fn build_source(names: &[String]) -> Option<(&'static str, String)> {
    let has = |wanted: &[&str]| {
        wanted
            .iter()
            .find_map(|w| names.iter().find(|n| n.eq_ignore_ascii_case(w)).cloned())
    };
    if let Some(f) = has(&["Dockerfile"]) {
        return Some(("dockerfile", f));
    }
    if let Some(f) = has(&[
        "environment.yml",
        "environment.yaml",
        "conda.yml",
        "conda.yaml",
        "env.yml",
        "env.yaml",
    ]) {
        return Some(("conda", f));
    }
    has(&["pyproject.toml", "setup.py"]).map(|f| ("pip", f))
}

/// The same, in plain words.
pub fn source_words(kind: &str) -> &'static str {
    match kind {
        "dockerfile" => "its own container recipe (Dockerfile)",
        "conda" => "its conda environment file",
        _ => "its Python package",
    }
}

/// Look at the top folder of a repository for something to build from.
pub fn detect(full: &str, branch: &str) -> Option<(&'static str, String)> {
    let listing = fetch_json(&format!("https://api.github.com/repos/{full}/contents?ref={branch}"))
        .ok()
        .flatten()?;
    let names: Vec<String> = listing
        .as_array()?
        .iter()
        .filter(|e| e["type"] == "file")
        .filter_map(|e| e["name"].as_str().map(String::from))
        .collect();
    build_source(&names)
}

/// The container recipe the app writes for a conda file or a Python package
/// (same shape as this project's own containers in containers/). The tool's
/// own package is installed in place (`pip install -e`) first, falling back to
/// a plain install: files a repository keeps beside its code, such as a
/// bundled database, are then where the tool looks for them.
fn recipe(kind: &str, file: &str) -> String {
    if kind == "conda" {
        format!(
            "FROM mambaorg/micromamba:2.0.5\n\
             COPY {file} /tmp/environment.yml\n\
             RUN micromamba install -y -n base -f /tmp/environment.yml && micromamba clean -a -y\n\
             USER root\n\
             COPY . /opt/tool-src\n\
             # the tool itself, when it is a Python package its environment file does not list\n\
             RUN if [ -x /opt/conda/bin/python ]; then if [ -f /opt/tool-src/pyproject.toml ] || [ -f /opt/tool-src/setup.py ]; then \\\n\
             \x20     /opt/conda/bin/python -m pip install --no-cache-dir --no-deps -e /opt/tool-src \\\n\
             \x20     || /opt/conda/bin/python -m pip install --no-cache-dir --no-deps /opt/tool-src \\\n\
             \x20     || echo 'the repository itself was not installed'; fi; fi\n\
             ENV PATH=/opt/conda/bin:$PATH\n\
             ENTRYPOINT []\n"
        )
    } else {
        "FROM python:3.12-slim\n\
         RUN apt-get update && apt-get install -y --no-install-recommends git build-essential \\\n\
         \x20   && rm -rf /var/lib/apt/lists/*\n\
         COPY . /opt/tool-src\n\
         RUN pip install --no-cache-dir -e /opt/tool-src || pip install --no-cache-dir /opt/tool-src\n\
         ENTRYPOINT []\n"
            .to_string()
    }
}

/// Unpack a repository's archive into `dest` (without its top folder).
pub(crate) fn unpack(url: &str, dest: &Path) -> Result<()> {
    let resp = ureq::get(url)
        .set("User-Agent", AGENT)
        .timeout(Duration::from_secs(900))
        .call()
        .with_context(|| format!("downloading {url}"))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(resp.into_reader()));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let rel: PathBuf = path.components().skip(1).collect();
        // never outside the folder
        if rel.as_os_str().is_empty() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            continue;
        }
        let kind = entry.header().entry_type();
        let to = dest.join(&rel);
        if kind.is_dir() {
            fs::create_dir_all(&to)?;
        } else if kind.is_file() {
            if let Some(p) = to.parent() {
                fs::create_dir_all(p)?;
            }
            entry.unpack(&to)?;
        }
    }
    Ok(())
}

/// `docker build`, with its progress in the job's log.
fn docker_build(tag: &str, recipe: &Path, context: &Path, r: &Reporter) -> Result<()> {
    // one stream for both: the older builder prints its steps on stdout, the newer on stderr
    let mut cmd = Command::new("sh");
    gwm_engine::docker::apply(&mut cmd);
    let mut child = cmd
        .args(["-c", "exec docker build \"$@\" 2>&1", "sh", "-t", tag, "-f"])
        .arg(recipe)
        .arg(context)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("starting docker")?;
    let mut last: Vec<String> = Vec::new();
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            r.progress_line(&line);
            last.push(line);
            if last.len() > 25 {
                last.remove(0);
            }
        }
    }
    if !child.wait()?.success() {
        bail!("the container could not be built:\n{}", last.join("\n"));
    }
    Ok(())
}

/// Build a tool's container from its repository and put its address in the tool.
pub fn build(cat: &Catalog, id: &str, r: &Reporter) -> Result<()> {
    let tool = own_tool(cat, id)?;
    let text = |block: &str, key: &str| {
        tool.extra
            .get(block)
            .and_then(|b| b.get(key))
            .and_then(Value::as_str)
            .map(String::from)
    };
    let (Some(kind), Some(file)) = (text("build", "kind"), text("build", "file")) else {
        bail!(
            "{} has nothing to build a container from: type its container address instead",
            tool.name
        );
    };
    let (Some(full), Some(branch)) = (text("github", "repo"), text("github", "branch")) else {
        bail!("{} was not added from GitHub", tool.name);
    };
    if gwm_engine::backend::detect().name() != "docker" {
        bail!("building a container needs Docker, and this computer runs its tools with Apptainer: type a container address instead");
    }
    let commit = fetch_json(&format!("https://api.github.com/repos/{full}/commits/{branch}"))?
        .and_then(|c| c["sha"].as_str().map(String::from))
        .ok_or_else(|| anyhow!("GitHub did not say which commit {branch} is at"))?;
    let dir = cat.home.join("build").join(id);
    let _ = fs::remove_dir_all(&dir);
    let src = dir.join("src");
    fs::create_dir_all(&src)?;
    r.log(format!("Downloading {full} (commit {})", &commit[..7]));
    unpack(&format!("https://codeload.github.com/{full}/tar.gz/{commit}"), &src)?;

    let version: String = tool
        .version
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || ".-_".contains(*c))
        .collect();
    let tag = format!(
        "gwm-local/{}:{}{}",
        id.replace('_', "-"),
        if version.is_empty() {
            String::new()
        } else {
            format!("{}-", version.to_lowercase())
        },
        &commit[..7]
    );
    r.log(format!(
        "Building the container from {} (this takes several minutes the first time)",
        source_words(&kind)
    ));
    if kind == "dockerfile" {
        // the repository's own recipe, then a thin layer so the app can run its script in it
        let base = format!("{tag}-base");
        docker_build(&base, &src.join(&file), &src, r)?;
        let wrap = dir.join("wrap");
        fs::create_dir_all(&wrap)?;
        fs::write(
            wrap.join("Dockerfile"),
            format!("FROM {base}\nUSER root\nENTRYPOINT []\n"),
        )?;
        docker_build(&tag, &wrap.join("Dockerfile"), &wrap, r)?;
    } else {
        let recipe_file = dir.join("Dockerfile.gwm");
        fs::write(&recipe_file, recipe(&kind, &file))?;
        docker_build(&tag, &recipe_file, &src, r)?;
    }
    // the repository is in hand: keep its test files, to try the tool on
    crate::selftest::keep_test_files(cat, id, &src);
    let _ = fs::remove_dir_all(&dir);

    let path = tool.dir.join("tool.yml");
    let Value::Mapping(mut raw) = load_yaml(&path)? else {
        bail!("{}: not a tool definition", path.display())
    };
    raw.insert(y("container"), y(&tag));
    let mut built = Mapping::new();
    built.insert(y("from"), y(&kind));
    built.insert(y("commit"), y(&commit));
    raw.insert(y("built"), Value::Mapping(built));
    dump_yaml(&raw, &path)?;
    r.log(format!("Built: {tag}"));
    // a container that builds is not yet a tool that works: try it
    crate::selftest::check_after_build(cat, id, r);
    r.result(json!({"container": tag}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_repository_builds_from() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            build_source(&names(&["README.md", "setup.py", "environment.yml", "Dockerfile"])),
            Some(("dockerfile", "Dockerfile".into()))
        );
        assert_eq!(
            build_source(&names(&["pyproject.toml", "conda.yaml"])),
            Some(("conda", "conda.yaml".into()))
        );
        assert_eq!(build_source(&names(&["setup.py"])), Some(("pip", "setup.py".into())));
        assert_eq!(build_source(&names(&["Makefile", "main.c"])), None);
    }

    #[test]
    fn written_recipes() {
        let conda = recipe("conda", "env.yml");
        assert!(conda.starts_with("FROM mambaorg/micromamba:") && conda.contains("COPY env.yml /tmp/environment.yml"));
        assert!(conda.ends_with("ENTRYPOINT []\n"));
        let pip = recipe("pip", "setup.py");
        assert!(pip.starts_with("FROM python:") && pip.contains("pip install --no-cache-dir /opt/tool-src"));
    }
}
