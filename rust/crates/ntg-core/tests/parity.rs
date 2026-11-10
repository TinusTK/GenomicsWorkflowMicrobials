//! The Rust compiler must give the same step order, settings and runnable map as
//! the v1 Python compiler (fixtures from rust/tools/make_parity_fixtures.py).

use std::fs;
use std::path::{Path, PathBuf};

use ntg_core::{compiler, Catalog};
use serde_json::Value;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap()
        .to_path_buf()
}

fn catalog() -> Catalog {
    // empty home: built-in tools and workflows only
    let home = std::env::temp_dir().join(format!("ntg-test-empty-{}", std::process::id()));
    Catalog::new(repo_root(), home)
}

#[test]
fn rdh_v5_2_matches_python() {
    let fixture: Value = serde_json::from_str(
        &fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rdh_v5_2.json")).unwrap(),
    )
    .unwrap();
    let cat = catalog();
    let tools = cat.load_tools().unwrap();
    let wf = cat.load_workflow("rdh_v5_2", None).unwrap();

    let resolved = compiler::resolve(&wf, &tools).unwrap();
    let order: Vec<&str> = resolved.iter().map(|r| r.id.as_str()).collect();
    let want: Vec<&str> = fixture["order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(order, want);

    let plan = compiler::plan(&wf, &tools, &cat.load_databases().unwrap()).unwrap();
    for s in &plan.steps {
        let want = &fixture["settings"][&s.id];
        let got = serde_json::to_value(&s.settings).unwrap();
        assert_eq!(&got, want, "settings of step {}", s.id);
        assert_eq!(Value::Bool(s.runnable), fixture["runnable"][&s.id], "runnable {}", s.id);
    }
}

#[test]
fn plus_needs_its_packs() {
    let cat = catalog();
    let tools = cat.load_tools().unwrap();
    let wf = cat.load_workflow("rdh_v5_2_plus", None).unwrap();
    assert_eq!(wf.steps.len(), 19, "extends: parent steps + own");
    let e = compiler::resolve(&wf, &tools).unwrap_err();
    assert_eq!(
        e.to_string(),
        "step shigella_mykrobe: tool 'shigella_mykrobe' is not installed"
    );
}

#[test]
fn step_off_skips_dependents() {
    let cat = catalog();
    let tools = cat.load_tools().unwrap();
    let mut wf = cat.load_workflow("rdh_v5_2", None).unwrap();
    wf.steps.get_mut("kraken2").unwrap().enabled = Some(false);
    let run = compiler::runnable(&wf, &tools).unwrap();
    assert!(!run["kraken2"]);
    assert!(!run["bracken"], "bracken reads kraken2's report");
    assert!(run["summary"], "summary inputs are optional");
}

#[test]
fn loops_are_refused() {
    let cat = catalog();
    let tools = cat.load_tools().unwrap();
    let mut wf = cat.load_workflow("rdh_v5_2", None).unwrap();
    wf.steps
        .get_mut("fastp")
        .unwrap()
        .inputs
        .insert("reads".into(), vec!["shovill.contigs".into()]);
    let e = compiler::resolve(&wf, &tools).unwrap_err().to_string();
    assert!(e.starts_with("steps form a loop"), "{e}");
}
