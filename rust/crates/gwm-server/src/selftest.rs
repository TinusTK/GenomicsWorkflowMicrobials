//! Trying a tool on its own repository's test files.
//!
//! A tool added from GitHub is only known to work once it has run on real
//! input. Most repositories carry a small test genome or a pair of reads
//! (`tests/data`, `test-data`, `examples`, ...): the app keeps a copy when it
//! has the repository in hand (a container build) or fetches it, and tries the
//! tool on it: right after a build, and in "Set it up for me" when no folder
//! was given. A tool that does not work (a database that was left out of its
//! container, say) then shows its own error at once.

use std::fs;
use std::path::{Path, PathBuf};

use gwm_core::catalog::Tool;
use gwm_core::reads::{find_samples, is_fasta};
use gwm_core::Catalog;
use indexmap::IndexMap;
use serde_yaml::Value;

use crate::jobs::Reporter;
use crate::tooledit::{own_tool, run_try, TryRequest};

/// Folder names that hold test or example files (a name starting with one counts).
const TEST_FOLDERS: [&str; 5] = ["test", "example", "demo", "fixture", "tutorial"];
/// A test genome is at least this big: smaller FASTA files are edge cases
/// (empty, no loci) that a tool is right to stop on.
const MIN_FASTA: u64 = 1_000;
const MAX_FILE: u64 = 500 * 1024 * 1024;
/// A repository with more files than this is not walked to the end.
const MAX_ENTRIES: usize = 20_000;

/// Where a tool's kept test files are: `<data folder>/try-data/<id>/{fasta,reads}`.
pub fn data_dir(cat: &Catalog, id: &str) -> PathBuf {
    cat.home.join("try-data").join(id)
}

/// Every folder under `root` that is, or is inside, a test or example folder.
fn test_dirs(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, inside: bool, seen: &mut usize, out: &mut Vec<PathBuf>) {
        if inside {
            out.push(dir.to_path_buf());
        }
        if depth == 0 || *seen > MAX_ENTRIES {
            return;
        }
        let Ok(rd) = fs::read_dir(dir) else { return };
        let mut subs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        subs.sort();
        for sub in subs {
            *seen += 1;
            let name = sub.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let testish = TEST_FOLDERS.iter().any(|t| name.starts_with(t));
            walk(&sub, depth - 1, inside || testish, seen, out);
        }
    }
    let mut out = Vec::new();
    walk(root, 6, false, &mut 0, &mut out);
    out
}

fn size(p: &Path) -> u64 {
    fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// The biggest FASTA file among the repository's test files: the one most
/// likely to be a whole genome.
fn pick_fasta(root: &Path) -> Option<PathBuf> {
    test_dirs(root)
        .iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flat_map(|rd| rd.flatten().map(|e| e.path()))
        .filter(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(is_fasta))
        .filter(|p| (MIN_FASTA..=MAX_FILE).contains(&size(p)))
        .max_by_key(|p| size(p))
}

/// The biggest complete pair of read files among the repository's test files.
fn pick_reads(root: &Path) -> Option<(PathBuf, PathBuf)> {
    test_dirs(root)
        .iter()
        .flat_map(|d| find_samples(d, None).samples)
        .filter_map(|s| Some((s.r1, s.r2?)))
        .filter(|(a, b)| size(a) > 0 && size(a) <= MAX_FILE && size(b) <= MAX_FILE)
        .max_by_key(|(a, _)| size(a))
}

/// Keep a copy of the test files of the repository unpacked in `src`, for
/// trying the tool later. Whatever was kept before is replaced.
pub fn keep_test_files(cat: &Catalog, id: &str, src: &Path) {
    let dir = data_dir(cat, id);
    let _ = fs::remove_dir_all(&dir);
    let copy = |from: &Path, to: &str| -> Option<()> {
        let dest = dir.join(to);
        fs::create_dir_all(&dest).ok()?;
        fs::copy(from, dest.join(from.file_name()?)).ok().map(|_| ())
    };
    if let Some(f) = pick_fasta(src) {
        let _ = copy(&f, "fasta");
    }
    if let Some((a, b)) = pick_reads(src) {
        let _ = copy(&a, "reads");
        let _ = copy(&b, "reads");
    }
}

/// `owner/repo` and branch of a tool added from GitHub.
fn github(tool: &Tool) -> Option<(String, String)> {
    let g = tool.extra.get("github")?;
    Some((
        g.get("repo").and_then(Value::as_str)?.to_string(),
        g.get("branch").and_then(Value::as_str)?.to_string(),
    ))
}

/// What to try this tool on, from its own repository's test files: the
/// request for the try-out, and the file's name for the log. The repository is
/// fetched when no copy of its test files was kept yet. Nothing when the tool
/// takes neither reads nor an assembly, or its repository has no such file.
pub fn test_input(cat: &Catalog, tool: &Tool, r: &Reporter) -> Option<(TryRequest, String)> {
    let reads = tool.inputs.values().any(|i| i.kind == "reads");
    let fasta_input = tool
        .inputs
        .iter()
        .find(|(_, i)| i.kind == "fasta")
        .map(|(n, _)| n.clone());
    if !reads && fasta_input.is_none() {
        return None;
    }
    let dir = data_dir(cat, &tool.id);
    if !dir.is_dir() {
        let (full, branch) = github(tool)?;
        r.log("Getting the repository's own test files");
        let tmp = cat.home.join("build").join(format!("{}-tests", tool.id));
        let _ = fs::remove_dir_all(&tmp);
        let got = fs::create_dir_all(&tmp)
            .map_err(anyhow::Error::from)
            .and_then(|_| crate::build::unpack(&format!("https://codeload.github.com/{full}/tar.gz/{branch}"), &tmp));
        if got.is_ok() {
            keep_test_files(cat, &tool.id, &tmp);
        }
        let _ = fs::remove_dir_all(&tmp);
        // looked once: an empty folder says "this repository has none"
        let _ = fs::create_dir_all(&dir);
    }
    let name = |p: &Path| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
    if reads {
        let folder = dir.join("reads");
        let first = find_samples(&folder, None)
            .samples
            .into_iter()
            .find(|s| s.r2.is_some())?;
        let req = TryRequest {
            reads_folder: folder.to_string_lossy().into_owned(),
            ..Default::default()
        };
        return Some((req, name(&first.r1)));
    }
    let file = fs::read_dir(dir.join("fasta"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file())?;
    let req = TryRequest {
        files: IndexMap::from([(fasta_input?, file.to_string_lossy().into_owned())]),
        ..Default::default()
    };
    Some((req, name(&file)))
}

/// After a container build: does the tool work in it? Tried on the
/// repository's own test file when the tool has a command and such a file.
/// Only reports: a container that was built stays built.
pub fn check_after_build(cat: &Catalog, id: &str, r: &Reporter) {
    let Ok(tool) = own_tool(cat, id) else { return };
    if tool.command.trim().is_empty() {
        return; // nothing to run yet: "Set it up for me" tries it
    }
    let Some((req, what)) = test_input(cat, &tool, r) else {
        r.log("The repository has no test file the app can try the tool on: try it yourself with Try on one sample.");
        return;
    };
    r.log(format!(
        "Checking the tool works, on the repository's own test file ({what})"
    ));
    match run_try(cat, &tool, &req) {
        Ok(done) if done.exit == 0 => r.log("It works: the tool ran to the end on it."),
        Ok(done) => {
            let lines: Vec<&str> = done.log.lines().filter(|l| !l.trim().is_empty()).collect();
            r.log(format!(
                "The container was built, but the tool stopped with an error on it. What it printed last:\n{}",
                lines[lines.len().saturating_sub(6)..].join("\n")
            ));
            let _ = fs::remove_dir_all(&done.dir);
        }
        Err(e) => r.log(format!("The tool could not be tried: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_files_of_a_repository() {
        let root = std::env::temp_dir().join(format!("gwm-selftest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let put = |rel: &str, bytes: usize| {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, vec![b'A'; bytes]).unwrap();
        };
        put("database/all_schemes.fasta", 90_000); // not a test file, however big
        put("tests/data/empty.fa", 0);
        put("tests/data/none.fa", 200); // an edge case, too small to be a genome
        put("tests/data/example.fna.gz", 5_000);
        put("galaxywrapper/test-data/saureus.fasta", 40_000);
        put("examples/reads/s1_R1.fastq.gz", 3_000);
        put("examples/reads/s1_R2.fastq.gz", 3_000);
        put("examples/reads/lonely_R1.fastq.gz", 9_000); // no partner
        put("src/tool.py", 100);

        assert_eq!(
            pick_fasta(&root),
            Some(root.join("galaxywrapper/test-data/saureus.fasta"))
        );
        assert_eq!(
            pick_reads(&root),
            Some((
                root.join("examples/reads/s1_R1.fastq.gz"),
                root.join("examples/reads/s1_R2.fastq.gz")
            ))
        );
        // a repository with neither
        let bare = root.join("src");
        assert_eq!(pick_fasta(&bare), None);
        assert_eq!(pick_reads(&bare), None);
        fs::remove_dir_all(&root).ok();
    }
}
