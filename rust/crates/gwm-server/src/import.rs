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
