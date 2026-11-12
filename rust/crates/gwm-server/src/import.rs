//! The internet look-ups a Galaxy workflow import needs (BioContainers on
//! quay.io, the Galaxy Tool Shed), and the per-computer choice of which tool
//! outputs the Run page shows.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use gwm_core::galaxy::{Lookup, ShedInfo, ShedRef};
use gwm_core::Catalog;
use serde_json::Value;

const QUAY_API: &str = "https://quay.io/api/v1/repository/biocontainers";
const TOOL_SHED: &str = "https://toolshed.g2.bx.psu.edu";

fn get_json(url: &str) -> Result<Value> {
    let text = ureq::get(url)
        .timeout(Duration::from_secs(30))
        .call()
        .with_context(|| format!("asking {url}"))?
        .into_string()?;
    Ok(serde_json::from_str(&text)?)
}

/// Text safe inside a URL query.
fn q(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || "._-+".contains(*c))
        .collect::<String>()
        .replace('+', "%2B")
}

/// The numbers in a version (`1.4.2` -> 1, 4, 2), to tell which is newer.
pub fn version_key(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// Every version of a BioContainers package as (version, image of its newest
/// build), newest version first.
pub fn biocontainer_versions(package: &str) -> Result<Vec<(String, String)>> {
    let mut best: BTreeMap<String, (u32, String)> = BTreeMap::new();
    for page in 1..=20 {
        let d = get_json(&format!(
            "{QUAY_API}/{}/tag/?limit=100&onlyActiveTags=true&page={page}",
            q(package)
        ))?;
        for tag in d["tags"].as_array().into_iter().flatten() {
            let Some(name) = tag["name"].as_str() else { continue };
            let Some((version, build)) = name.split_once("--") else {
                continue;
            };
            let n = build.rsplit('_').next().and_then(|n| n.parse().ok()).unwrap_or(0);
            if best.get(version).is_none_or(|(b, _)| n >= *b) {
                best.insert(version.to_string(), (n, name.to_string()));
            }
        }
        if d["has_additional"].as_bool() != Some(true) {
            break;
        }
    }
    let mut out: Vec<(String, String)> = best
        .into_iter()
        .map(|(v, (_, tag))| (v, format!("quay.io/biocontainers/{package}:{tag}")))
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(version_key(&a.0)));
    Ok(out)
}

/// `ghcr.io/owner/name:tag` -> (`owner/name`, `tag`): a container on GitHub's
/// registry, where the app's own tools (and many others) publish theirs.
pub fn ghcr_image(container: &str) -> Option<(&str, &str)> {
    let (repo, tag) = container.strip_prefix("ghcr.io/")?.split_once(':')?;
    (!repo.is_empty() && !tag.is_empty() && !tag.contains('/')).then_some((repo, tag))
}

/// The tags of a list that are versions (they start with a digit), as
/// (version, image), newest first.
fn version_tags(repo: &str, tags: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = tags
        .iter()
        .filter(|t| t.starts_with(|c: char| c.is_ascii_digit()))
        .map(|t| (t.clone(), format!("ghcr.io/{repo}:{t}")))
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(version_key(&a.0)));
    out
}

/// Every published version of a public container on GitHub's registry as
/// (version, image), newest first. Nothing is downloaded but the list of tags.
pub fn ghcr_versions(repo: &str) -> Result<Vec<(String, String)>> {
    let repo = repo.to_ascii_lowercase();
    let token = get_json(&format!(
        "https://ghcr.io/token?service=ghcr.io&scope=repository:{repo}:pull"
    ))?;
    let token = token["token"].as_str().context("GitHub's registry gave no access")?;
    let url = format!("https://ghcr.io/v2/{repo}/tags/list?n=1000");
    let text = ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(30))
        .call()
        .with_context(|| format!("asking {url}"))?
        .into_string()?;
    let d: Value = serde_json::from_str(&text)?;
    let tags: Vec<String> = d["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().map(str::to_string))
        .collect();
    Ok(version_tags(&repo, &tags))
}

/// Looks things up on quay.io and the Galaxy Tool Shed.
pub struct Online;

impl Lookup for Online {
    /// The newest build of `<package>:<version>--<build>` on BioContainers.
    fn biocontainer(&self, package: &str, version: &str) -> Result<Option<String>> {
        let mut best: Option<(u32, String)> = None;
        for page in 1..=5 {
            let d = get_json(&format!(
                "{QUAY_API}/{}/tag/?limit=100&onlyActiveTags=true&page={page}",
                q(package)
            ))?;
            for tag in d["tags"].as_array().into_iter().flatten() {
                let Some(name) = tag["name"].as_str() else { continue };
                let Some(build) = name.strip_prefix(&format!("{version}--")) else {
                    continue;
                };
                let n = build.rsplit('_').next().and_then(|n| n.parse().ok()).unwrap_or(0);
                if best.as_ref().is_none_or(|(b, _)| n > *b) {
                    best = Some((n, name.to_string()));
                }
            }
            if d["has_additional"].as_bool() != Some(true) {
                break;
            }
        }
        Ok(best.map(|(_, tag)| format!("quay.io/biocontainers/{package}:{tag}")))
    }

    fn shed_tool(&self, shed: &ShedRef) -> Result<Option<ShedInfo>> {
        let d = get_json(&format!(
            "{TOOL_SHED}/api/repositories/get_repository_revision_install_info?name={}&owner={}&changeset_revision={}",
            q(&shed.name),
            q(&shed.owner),
            q(&shed.changeset_revision)
        ))?;
        let (Some(repo), Some(revision)) = (d.get(0), d.get(1)) else {
            return Ok(None);
        };
        let tools = revision["valid_tools"].as_array().cloned().unwrap_or_default();
        let Some(tool) = tools
            .iter()
            .find(|t| t["id"].as_str() == Some(&shed.tool_id))
            .or(tools.first())
        else {
            return Ok(None);
        };
        let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
        let mut info = ShedInfo {
            name: text(&tool["name"]),
            description: text(&tool["description"]),
            homepage: text(&repo["homepage_url"]),
            category: String::new(),
            requirements: tool["requirements"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["type"].as_str() == Some("package"))
                .map(|r| (text(&r["name"]), text(&r["version"])))
                .collect(),
        };
        // the category is nice to have: never fail the import over it
        if let Some(id) = repo["id"].as_str() {
            let cats = get_json(&format!("{TOOL_SHED}/api/repositories/{}", q(id)))
                .ok()
                .and_then(|r| r["category_ids"].as_array().cloned())
                .unwrap_or_default();
            if let (Some(first), Ok(all)) = (cats.first(), get_json(&format!("{TOOL_SHED}/api/categories"))) {
                if let Some(c) = all.as_array().into_iter().flatten().find(|c| c["id"] == *first) {
                    info.category = text(&c["name"]);
                }
            }
        }
        Ok(Some(info))
    }
}

// ---------------------------------------------------------------- outputs shown on the Run page

/// Kept with a tool's switched-off outputs: its text and JSON reports are not
/// turned into tables on the Run page (they are, unless this is there).
pub use gwm_core::catalog::REPORT_TABLES;

fn outputs_file(cat: &Catalog) -> PathBuf {
    cat.home.join("tool_outputs.json")
}

/// {tool: outputs switched off on the Tools page}; every output shows by default.
pub fn hidden_outputs(cat: &Catalog) -> BTreeMap<String, Vec<String>> {
    fs::read_to_string(outputs_file(cat))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn set_hidden_outputs(cat: &Catalog, tool: &str, hidden: Vec<String>) -> Result<()> {
    let mut all = hidden_outputs(cat);
    if hidden.is_empty() {
        all.remove(tool);
    } else {
        all.insert(tool.to_string(), hidden);
    }
    fs::create_dir_all(&cat.home)?;
    fs::write(outputs_file(cat), serde_json::to_string_pretty(&all)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_images_and_tags() {
        assert_eq!(
            ghcr_image("ghcr.io/tinustk/gwm-mlst-typing:0.3.0"),
            Some(("tinustk/gwm-mlst-typing", "0.3.0"))
        );
        assert_eq!(ghcr_image("quay.io/biocontainers/shovill:1.4.2--hdfd78af_0"), None);
        assert_eq!(ghcr_image("ghcr.io/tinustk/gwm-mlst-typing"), None);
        let tags: Vec<String> = ["0.1.0", "latest", "0.10.0", "0.3.0", "sha-abc"]
            .iter()
            .map(|t| t.to_string())
            .collect();
        let found = version_tags("o/n", &tags);
        let versions: Vec<&str> = found.iter().map(|(v, _)| v.as_str()).collect();
        assert_eq!(versions, ["0.10.0", "0.3.0", "0.1.0"]);
        assert_eq!(found[0].1, "ghcr.io/o/n:0.10.0");
    }
}
