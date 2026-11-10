//! The Rust compiler must give the same step order, settings and runnable map as
//! the v1 Python compiler (fixtures made from v1 before it was removed).

use std::fs;
use std::path::{Path, PathBuf};

use gwm_core::{compiler, Catalog, Workflow};
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
    let home = std::env::temp_dir().join(format!("gwm-test-empty-{}", std::process::id()));
    Catalog::new(repo_root(), home)
}

/// The v1 default workflow (built in up to 2.0.0-rc.1), kept for the parity fixtures.
fn rdh_v5_2() -> Workflow {
    let text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workflows/rdh_v5_2.yml"))
        .unwrap();
    serde_yaml::from_str(&text).unwrap()
}

#[test]
fn rdh_v5_2_matches_python() {
    let fixture: Value = serde_json::from_str(
        &fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rdh_v5_2.json")).unwrap(),
    )
    .unwrap();
    let cat = catalog();
    let tools = cat.load_tools().unwrap();
    let wf = rdh_v5_2();

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
        // v1 had one Kraken2 database: the Database setting came after it
        let mut settings = s.settings.clone();
        settings.shift_remove("database");
        let got = serde_json::to_value(&settings).unwrap();
        assert_eq!(&got, want, "settings of step {}", s.id);
        assert_eq!(Value::Bool(s.runnable), fixture["runnable"][&s.id], "runnable {}", s.id);
    }
}

#[test]
fn rdh_v6_is_built_in_and_runs() {
    let cat = catalog();
    let tools = cat.load_tools().unwrap();
    let wf = cat.load_workflow("rdh_v6", None).unwrap();
    assert!(wf.builtin);
    assert_eq!(wf.steps.len(), 17);
    let plan = compiler::plan(&wf, &tools, &cat.load_databases().unwrap()).unwrap();
    assert!(
        plan.steps.iter().all(|s| s.runnable),
        "every step of the default workflow runs"
    );
    let shig = plan.steps.iter().find(|s| s.id == "shigella_typing").unwrap();
    assert_eq!(shig.tool_version, "1.0.2");
    assert_eq!(shig.publish, "summary/shigella");
}

#[test]
fn step_off_skips_dependents() {
    let cat = catalog();
    let tools = cat.load_tools().unwrap();
    let mut wf = rdh_v5_2();
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
    let mut wf = rdh_v5_2();
    wf.steps
        .get_mut("fastp")
        .unwrap()
        .inputs
        .insert("reads".into(), vec!["shovill.contigs".into()]);
    let e = compiler::resolve(&wf, &tools).unwrap_err().to_string();
    assert!(e.starts_with("steps form a loop"), "{e}");
}
