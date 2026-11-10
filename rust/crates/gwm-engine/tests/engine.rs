//! Runs a small two-tool workflow with plain bash (no containers).
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use gwm_core::reads::find_samples;
use gwm_core::{compiler, Catalog};
use gwm_engine::backend::Local;
use gwm_engine::runs::{self, StartOptions};
use gwm_engine::{Executor, Limits, RunSpec, Status};

fn setup(name: &str) -> (Catalog, PathBuf) {
    let base = std::env::temp_dir().join(format!("gwm-engine-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let root = base.join("root");
    let w = |p: &str, text: &str| {
        let f = root.join(p);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, text).unwrap();
    };
    w("databases/catalog.yml", "databases: {}\n");
    w(
        "tools/count/tool.yml",
        r#"id: count
name: Count
version: 1.0
container: none
resources: {cpus: 1, memory_gb: 1}
inputs:
  reads: {type: reads}
outputs:
  report: {type: tsv, path: "out/{sample}.count.tsv"}
  extra: {type: txt, path: "out/{sample}.extra.txt", optional: true}
settings:
  word: {type: string, default: "it's"}
versions:
  count: echo 9.9
command: |
  echo "low_memory: no" >> .ntg_notes
  printf '%s\t%s\t%s\n' "$SAMPLE" "$(( $(cat "$IN_reads_1" "$IN_reads_2" | wc -l) ))" "$S_word" > "out/$SAMPLE.count.tsv"
  [ "$SAMPLE" = A ] && touch "out/$SAMPLE.extra.txt" || true
"#,
    );
    w(
        "tools/extra/tool.yml",
        r#"id: extra
name: Extra
version: 1
container: none
inputs:
  e: {type: txt}
outputs:
  done: {type: txt, path: "out/{sample}.done"}
command: touch "out/$SAMPLE.done"
"#,
    );
    w(
        "tools/combine/tool.yml",
        r#"id: combine
name: Combine
version: 1
container: none
scope: all_samples
files: [helper.sh]
inputs:
  counts: {type: tsv}
  opt: {type: txt, optional: true}
outputs:
  table: {type: tsv, path: out/table.tsv}
  opt: {type: txt, path: out/opt.txt}
command: |
  bash "$TOOL_DIR/helper.sh" "$IN_counts_DIR" > out/table.tsv
  ls "$IN_opt_DIR" > out/opt.txt
"#,
    );
    w("tools/combine/helper.sh", "cat \"$1\"/* | sort\n");
    w(
        "workflows/mini/workflow.yml",
        r#"id: mini
name: Mini
steps:
  count: {tool: count, inputs: {reads: [input]}}
  extra: {tool: extra, inputs: {e: [count.extra]}}
  combine: {tool: combine, publish: summary, inputs: {counts: [count.report], opt: [extra.done]}}
"#,
    );
    let reads = base.join("reads");
    fs::create_dir_all(&reads).unwrap();
    for s in ["A", "B10", "B2"] {
        fs::write(reads.join(format!("{s}_R1.fq.gz")), "x\ny\n").unwrap();
        fs::write(reads.join(format!("{s}_R2.fq.gz")), "z\n").unwrap();
    }
    (Catalog::new(root, base.join("home")), reads)
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn runs_workflow_and_writes_results() {
    let (cat, reads) = setup("run");
    let stop = AtomicBool::new(false);
    let run = runs::start(&cat, "mini", &reads, &StartOptions::default(), &Local, &stop, &|_| {}).unwrap();
    assert_eq!(run["status"], "succeeded", "{}", run["error"]);
    let results = PathBuf::from(run["results"].as_str().unwrap());
    assert_eq!(read(&results.join("samples/A/count/A.count.tsv")), "A\t3\tit's\n");
    assert_eq!(
        read(&results.join("summary/table.tsv")),
        "A\t3\tit's\nB10\t3\tit's\nB2\t3\tit's\n"
    );
    assert_eq!(
        read(&results.join("summary/opt.txt")),
        "A.done
",
        "optional input: the one file there is"
    );
    // optional output only for A: extra runs for A, is skipped for B2 and B10
    assert!(results.join("samples/A/extra/A.done").is_file());
    assert!(!results.join("samples/B2/extra").exists());
    assert_eq!(run["steps"]["extra"]["tasks"]["B2"]["status"], "SKIPPED");
    let versions = read(&results.join("pipeline_info/tool_versions.yml"));
    assert!(versions.contains("count: 9.9"), "{versions}");
    assert!(versions.contains("note.count.low_memory: no"), "{versions}");
    let prov: serde_json::Value = serde_json::from_str(&read(&results.join("pipeline_info/provenance.json"))).unwrap();
    assert_eq!(prov["tools"]["count"], "9.9");
    assert_eq!(prov["containers"]["STEP_combine"]["image"], "none");
    assert_eq!(runs::list(&cat).len(), 1);
}

#[test]
fn step_off_and_resume() {
    let (cat, reads) = setup("resume");
    let tools = cat.load_tools().unwrap();
    let mut wf = cat.load_workflow("mini", None).unwrap();
    wf.steps["extra"].enabled = Some(false);
    let plan = compiler::plan(&wf, &tools, &cat.load_databases().unwrap()).unwrap();
    let run_dir = cat.home.join("runs/r1");
    for t in ["count", "extra", "combine"] {
        let to = run_dir.join("tools").join(t);
        fs::create_dir_all(&to).unwrap();
        for e in fs::read_dir(cat.root.join("tools").join(t)).unwrap().flatten() {
            fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
    let spec = RunSpec {
        run_id: "r1".into(),
        plan,
        samples: find_samples(&reads, None).samples,
        run_dir: run_dir.clone(),
        work_dir: cat.home.join("work/r1"),
        results_dir: run_dir.join("results"),
        db_dir: cat.home.join("databases"),
        limits: Limits {
            cpus: 2,
            memory_gb: 4,
            total_memory_gb: 8,
        },
    };
    let stop = AtomicBool::new(false);
    let first = Executor::new(&spec, &Local, &|_| {}).unwrap().run(&stop).unwrap();
    assert!(first.success, "{:?}", first.error);
    assert_eq!(first.ran, ["count", "combine"], "extra is switched off");
    let opt = run_dir.join("results/summary/opt.txt");
    assert_eq!(read(&opt), "", "optional input from a step that is off: empty folder");
    let second = Executor::new(&spec, &Local, &|_| {}).unwrap().run(&stop).unwrap();
    assert!(second.success);
    assert!(
        second.trace.iter().all(|r| r.status == Status::Cached),
        "{:?}",
        second.trace
    );
}

#[test]
fn failed_task_fails_the_run() {
    let (cat, reads) = setup("fail");
    let f = cat.root.join("tools/combine/helper.sh");
    fs::write(&f, "echo broken >&2; exit 3\n").unwrap();
    let stop = AtomicBool::new(false);
    let run = runs::start(&cat, "mini", &reads, &StartOptions::default(), &Local, &stop, &|_| {}).unwrap();
    assert_eq!(run["status"], "failed");
    let err = run["error"].as_str().unwrap();
    assert!(
        err.contains("STEP_combine (all) failed: exit status 3") && err.contains("broken"),
        "{err}"
    );
}
