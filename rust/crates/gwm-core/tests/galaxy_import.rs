//! Importing the Galaxy RDH workflows (reference/*.ga) must give the same
//! tools, versions, settings and wiring as the hand-written workflows.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use gwm_core::galaxy::{self, Import, Lookup, ShedInfo, ShedRef};
use gwm_core::{compiler, Catalog, Workflow};
use serde_json::Value as Json;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap()
        .to_path_buf()
}

/// A catalog with an empty data folder of its own.
fn catalog(name: &str) -> Catalog {
    let home = std::env::temp_dir().join(format!("gwm-import-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    Catalog::new(repo_root(), home)
}

fn ga(version: &str) -> String {
    let name = format!("Galaxy-Workflow-Genomic_Assembly_and_analysis_-_RDH_version_{version}.ga");
    fs::read_to_string(repo_root().join("reference").join(name)).unwrap()
}

fn import(cat: &Catalog, text: &str) -> Import {
    let tools = cat.load_tools().unwrap();
    galaxy::analyze(text, &tools, &|_| false).unwrap()
}

/// Same steps, each with the same tool, version, settings, wiring and results folder.
fn assert_same(cat: &Catalog, got: &Workflow, want: &Workflow) {
    let tools = cat.load_tools().unwrap();
    let dbs = cat.load_databases().unwrap();
    let got = compiler::plan(got, &tools, &dbs).unwrap();
    let want = compiler::plan(want, &tools, &dbs).unwrap();
    let ids = |p: &compiler::Plan| {
        let mut v: Vec<String> = p.steps.iter().map(|s| s.id.clone()).collect();
        v.sort();
        v
    };
    assert_eq!(ids(&got), ids(&want), "steps");
    for w in &want.steps {
        let g = got.steps.iter().find(|s| s.id == w.id).unwrap();
        assert_eq!(g.tool_id, w.tool_id, "tool of {}", w.id);
        assert_eq!(g.tool_version, w.tool_version, "version of {}", w.id);
        assert_eq!(g.container, w.container, "container of {}", w.id);
        assert_eq!(g.settings, w.settings, "settings of {}", w.id);
        assert_eq!(g.publish, w.publish, "results folder of {}", w.id);
        assert_eq!(g.runnable, w.runnable, "runnable {}", w.id);
        for (name, input) in &w.inputs {
            assert_eq!(g.inputs[name].sources, input.sources, "input {name} of {}", w.id);
        }
    }
}

#[test]
fn rdh_v6_import_matches_the_built_in() {
    let cat = catalog("v6");
    let imp = import(&cat, &ga("6"));
    assert!(imp.new_versions.is_empty(), "every tool version is in the app");
    assert!(imp.new_tools.is_empty(), "every tool is in the app");
    assert_same(&cat, &imp.workflow, &cat.load_workflow("rdh_v6", None).unwrap());

    assert_eq!(imp.workflow.name, "Genomic Assembly and analysis - RDH version 6");
    assert_eq!(imp.workflow.id, "genomic_assembly_and_analysis_rdh_version_6");
    let count = |a: &str| imp.steps.iter().filter(|s| s.action == a).count();
    assert_eq!(imp.steps.len(), 47);
    assert_eq!(count("builtin"), 16);
    assert_eq!(count("input"), 1);
    assert_eq!(count("summary"), 29);
    assert_eq!(count("left_out"), 1, "the plain copy of staramr's table");
    let checkm = &imp.workflow.steps["checkm"];
    assert_eq!(checkm.extra["galaxy_values"]["tree"], "full");
    assert_eq!(
        imp.workflow.steps["summary"].extra["galaxy_step"],
        "0, 2, 9, 17, 21-22, 24-46"
    );
}

#[test]
fn rdh_v5_2_import_matches_the_v1_workflow() {
    let cat = catalog("v52");
    let imp = import(&cat, &ga("5.2"));
    let text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workflows/rdh_v5_2.yml"))
        .unwrap();
    assert_same(&cat, &imp.workflow, &serde_yaml::from_str(&text).unwrap());
}

struct Fake;

impl Lookup for Fake {
    fn biocontainer(&self, package: &str, version: &str) -> Result<Option<String>> {
        Ok((version != "9.9.9").then(|| format!("quay.io/biocontainers/{package}:{version}--h1234_0")))
    }
    fn shed_tool(&self, shed: &ShedRef) -> Result<Option<ShedInfo>> {
        Ok(Some(ShedInfo {
            name: "snippy".into(),
            description: "Finds SNPs between a reference and your reads.".into(),
            homepage: "https://github.com/tseemann/snippy".into(),
            category: "Variant Analysis".into(),
            requirements: vec![(shed.name.clone(), "4.6.0".into())],
        }))
    }
}

/// The v6 file with one step's tool version changed.
fn with_version(step: &str, tool_version: &str) -> String {
    let mut g: Json = serde_json::from_str(&ga("6")).unwrap();
    let s = &mut g["steps"][step];
    let id = s["tool_id"].as_str().unwrap().rsplit_once('/').unwrap().0.to_string();
    s["tool_id"] = format!("{id}/{tool_version}").into();
    s["tool_version"] = tool_version.into();
    g.to_string()
}

#[test]
fn another_version_is_added_beside_the_built_in() {
    let cat = catalog("version");
    let imp = import(&cat, &with_version("4", "1.4.0+galaxy2"));
    assert_eq!(imp.new_versions.len(), 1);
    assert_eq!(
        (imp.new_versions[0].tool.as_str(), imp.new_versions[0].version.as_str()),
        ("shovill", "1.4.0")
    );
    let row = imp.steps.iter().find(|s| s.galaxy_step == 4).unwrap();
    assert_eq!(row.action, "new_version");

    let done = galaxy::apply(&cat, imp, &Fake).unwrap();
    let tools = cat.load_tools().unwrap();
    let added = &tools["shovill"].variants["1.4.0"];
    assert_eq!(added.container, "quay.io/biocontainers/shovill:1.4.0--h1234_0");
    assert_eq!(added.extra["untested"], true);
    let wf = cat.load_workflow(&done.id, None).unwrap();
    assert_eq!(wf.steps["shovill"].tool_version.as_deref(), Some("1.4.0"));
    let plan = compiler::plan(&wf, &tools, &cat.load_databases().unwrap()).unwrap();
    let shovill = plan.steps.iter().find(|s| s.id == "shovill").unwrap();
    assert_eq!(shovill.tool_version, "1.4.0");
    assert_eq!(shovill.tool_bundle, "shovill@1.4.0");

    // importing it again: the version is there now, and the name is taken
    let tools = cat.load_tools().unwrap();
    let again = galaxy::analyze(&with_version("4", "1.4.0+galaxy2"), &tools, &|id| {
        !cat.user_workflow_versions(id).is_empty()
    })
    .unwrap();
    assert!(again.new_versions.is_empty());
    assert_eq!(again.workflow.id, format!("{}_2", done.id));
}

#[test]
fn a_version_without_a_container_falls_back() {
    let cat = catalog("fallback");
    let imp = import(&cat, &with_version("7", "9.9.9"));
    let done = galaxy::apply(&cat, imp, &Fake).unwrap();
    let wf = cat.load_workflow(&done.id, None).unwrap();
    assert_eq!(wf.steps["mlst"].tool_version, None);
    let row = done.steps.iter().find(|s| s.galaxy_step == 7).unwrap();
    assert_eq!(row.action, "builtin");
    assert!(row.note.contains("could not be added"), "{}", row.note);
}

#[test]
fn an_unknown_tool_becomes_a_draft() {
    let cat = catalog("draft");
    let mut g: Json = serde_json::from_str(&ga("6")).unwrap();
    g["steps"]["47"] = serde_json::json!({
        "id": 47, "type": "tool", "name": "snippy", "label": null,
        "tool_id": "toolshed.g2.bx.psu.edu/repos/iuc/snippy/snippy/4.6.0+galaxy0",
        "tool_version": "4.6.0+galaxy0",
        "tool_shed_repository": {"owner": "iuc", "name": "snippy", "changeset_revision": "abc", "tool_shed": "toolshed.g2.bx.psu.edu"},
        "input_connections": {"fastq_input|fastq_input": {"id": 3, "output_name": "output_paired_coll"}},
        "outputs": [{"name": "snpvcf", "type": "vcf"}, {"name": "snptab", "type": "tabular"}],
        "tool_state": "{\"adv\": {\"mincov\": \"10\", \"rename_cons\": false}, \"fastq_input\": {\"fastq_input\": {\"__class__\": \"ConnectedValue\"}}, \"__page__\": null}",
    });
    let imp = import(&cat, &g.to_string());
    assert_eq!(imp.new_tools.len(), 1);
    assert_eq!(imp.new_tools[0].id, "snippy");
    let row = imp.steps.iter().find(|s| s.galaxy_step == 47).unwrap();
    assert_eq!((row.action.as_str(), row.step.as_deref()), ("new_tool", Some("snippy")));
    let step = &imp.workflow.steps["snippy"];
    assert_eq!(step.inputs["fastq_input"], ["fastp.reads", "input"]);

    let done = galaxy::apply(&cat, imp, &Fake).unwrap();
    let tools = cat.load_tools().unwrap();
    let t = &tools["snippy"];
    assert_eq!(t.source, galaxy::IMPORT_SOURCE);
    assert_eq!(t.version, "4.6.0");
    assert_eq!(t.container, "quay.io/biocontainers/snippy:4.6.0--h1234_0");
    assert_eq!(t.extra["category"], "Variant Analysis");
    assert_eq!(t.inputs["fastq_input"].kind, "reads");
    assert_eq!(t.outputs["snptab"].kind, "tsv");
    assert_eq!(t.settings["adv_mincov"].default, "10");
    assert_eq!(t.settings["adv_rename_cons"].default, false);

    // it shows in the builder, but can't run until its command is written
    let wf = cat.load_workflow(&done.id, None).unwrap();
    compiler::resolve(&wf, &tools).unwrap();
    let e = compiler::plan(&wf, &tools, &cat.load_databases().unwrap()).unwrap_err();
    assert!(e.to_string().contains("has no command yet"), "{e}");
}

#[test]
fn other_files_are_refused() {
    let cat = catalog("bad");
    let tools = cat.load_tools().unwrap();
    for text in [
        "not json",
        "{\"steps\": {}}",
        "{\"a_galaxy_workflow\": \"true\", \"steps\": {}}",
    ] {
        assert!(galaxy::analyze(text, &tools, &|_| false).is_err(), "{text}");
    }
}

#[test]
fn galaxy_versions() {
    assert_eq!(galaxy::tool_version("1.4.2+galaxy1"), "1.4.2");
    assert_eq!(galaxy::tool_version("2.22.0"), "2.22.0");
    assert_eq!(galaxy::tool_version("3.0+galaxy0"), "3.0");
}
