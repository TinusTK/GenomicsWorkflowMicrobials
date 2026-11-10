//! A run's samples from the folder it is given: read files paired up (port of
//! v1 `runs.find_samples`), or, for a workflow that starts from assemblies,
//! one sample per FASTA file.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

static READS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.(fastq|fq)(\.gz)?$|\.gz$").unwrap());
static FASTA_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.(fasta|fa|fna|fas|ffn)(\.gz)?$").unwrap());

/// A workflow's `input: type:` when it starts from assemblies (FASTA files);
/// anything else is paired reads.
pub const ASSEMBLIES: &str = "assemblies";
/// [`Found::mode`] of samples that are FASTA files.
pub const FASTA_MODE: &str = "fasta";

/// A FASTA file by its name: .fasta, .fa, .fna, .fas or .ffn, also with .gz.
pub fn is_fasta(name: &str) -> bool {
    FASTA_RE.is_match(name)
}

/// Tried in this order by Auto; the one that pairs the most files wins.
pub const PAIRINGS: [(&str, &str); 4] = [("_R1", "_R2"), (".R1", ".R2"), ("_1", "_2"), (".1", ".2")];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pairing {
    /// "auto" or "custom"
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub r1: String,
    #[serde(default)]
    pub r2: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    pub sample: String,
    /// the first read file, or the sample's FASTA file
    pub r1: PathBuf,
    /// None: no matching R2 file (the run can't start); always None for a FASTA file
    pub r2: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Found {
    pub samples: Vec<Sample>,
    pub unpaired: Vec<String>,
    pub r1: String,
    pub r2: String,
    pub mode: String,
}

/// Natural order ignoring case: sample2 before sample10.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    fn key(s: &str) -> Vec<(u8, u128, String)> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut digits = false;
        for c in s.chars() {
            if !cur.is_empty() && c.is_ascii_digit() != digits {
                out.push(part(&cur, digits));
                cur.clear();
            }
            digits = c.is_ascii_digit();
            cur.push(c);
        }
        if !cur.is_empty() {
            out.push(part(&cur, digits));
        }
        out
    }
    fn part(s: &str, digits: bool) -> (u8, u128, String) {
        match (digits, s.parse::<u128>()) {
            (true, Ok(n)) => (0, n, String::new()),
            _ => (1, 0, s.to_lowercase()),
        }
    }
    key(a).cmp(&key(b))
}

/// Start of the last `r1` in `name` that isn't followed by another digit
/// (so _R1 matches _R1_001 but not _R10).
fn last_marker(name: &str, r1: &str) -> Option<usize> {
    if r1.is_empty() {
        return None;
    }
    let mut found = None;
    let mut i = 0;
    while let Some(off) = name[i..].find(r1) {
        let start = i + off;
        let end = start + r1.len();
        if name[end..].chars().next().is_some_and(|c| c.is_ascii_digit()) {
            i = start + name[start..].chars().next().map_or(1, char::len_utf8);
        } else {
            found = Some(start);
            i = end;
        }
        if i >= name.len() {
            break;
        }
    }
    found
}

/// (sample, r1 file, r2 file if there is one)
type Pair = (String, String, Option<String>);

fn pair(names: &[String], r1: &str, r2: &str) -> (Vec<Pair>, Vec<String>) {
    let mut pairs = Vec::new();
    let mut used = Vec::new();
    for n in names {
        let Some(start) = last_marker(n, r1) else { continue };
        let partner = format!("{}{}{}", &n[..start], r2, &n[start + r1.len()..]);
        let head = &n[..start];
        let trimmed = head.trim_end_matches(['_', '.', '-']);
        let sample = if trimmed.is_empty() { head } else { trimmed };
        let has = names.contains(&partner);
        pairs.push((sample.to_string(), n.clone(), has.then(|| partner.clone())));
        used.push(n.clone());
        if has {
            used.push(partner);
        }
    }
    (pairs, used)
}

/// Pair the reads in `dir`. Returns exactly what goes into the run's samples.csv.
pub fn find_samples(dir: &Path, pairing: Option<&Pairing>) -> Found {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| READS_RE.is_match(n))
                .collect()
        })
        .unwrap_or_default();
    names.sort_by(|a, b| natural_cmp(a, b));
    let custom = pairing.filter(|p| p.mode == "custom" && !p.r1.is_empty() && !p.r2.is_empty());
    let tries: Vec<(&str, &str)> = match custom {
        Some(p) => vec![(p.r1.as_str(), p.r2.as_str())],
        None => PAIRINGS.to_vec(),
    };
    #[allow(clippy::type_complexity)]
    let mut best: Option<(usize, &str, &str, Vec<Pair>, Vec<String>)> = None;
    for (r1, r2) in &tries {
        let (pairs, used) = pair(&names, r1, r2);
        let score = pairs.iter().filter(|p| p.2.is_some()).count();
        if best.as_ref().is_none_or(|b| score > b.0) {
            best = Some((score, r1, r2, pairs, used));
        }
    }
    let (_, r1, r2, mut pairs, used) = best.unwrap();
    pairs.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    Found {
        samples: pairs
            .into_iter()
            .map(|(sample, a, b)| Sample {
                sample,
                r1: dir.join(a),
                r2: b.map(|b| dir.join(b)),
            })
            .collect(),
        unpaired: names.iter().filter(|n| !used.contains(n)).cloned().collect(),
        r1: r1.to_string(),
        r2: r2.to_string(),
        mode: if custom.is_some() { "custom" } else { "auto" }.to_string(),
    }
}

/// The FASTA files in `dir`, one sample each, named after the file.
pub fn find_assemblies(dir: &Path) -> Found {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| is_fasta(n))
                .collect()
        })
        .unwrap_or_default();
    names.sort_by(|a, b| natural_cmp(a, b));
    Found {
        samples: names
            .iter()
            .map(|n| Sample {
                sample: FASTA_RE.replace(n, "").into_owned(),
                r1: dir.join(n),
                r2: None,
            })
            .collect(),
        unpaired: Vec::new(),
        r1: String::new(),
        r2: String::new(),
        mode: FASTA_MODE.to_string(),
    }
}

/// The first FASTA file to try a tool on: `path` itself when it is one, else
/// the first in that folder.
pub fn first_fasta(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| is_fasta(n))
            .map(|_| path.to_path_buf());
    }
    find_assemblies(path).samples.into_iter().next().map(|s| s.r1)
}

/// The samples of a run from its input folder, for a workflow's input type.
pub fn find_input(dir: &Path, input_type: &str, pairing: Option<&Pairing>) -> Found {
    if input_type == ASSEMBLIES {
        find_assemblies(dir)
    } else {
        find_samples(dir, pairing)
    }
}

/// Plain error messages for samples that can't run (see [`find_input`]).
pub fn input_problems(found: &Found) -> Vec<String> {
    if found.mode == FASTA_MODE {
        same_names(found, "files")
    } else {
        pairing_problems(found)
    }
}

/// Plain error messages for pairs that can't run.
pub fn pairing_problems(found: &Found) -> Vec<String> {
    let mut out = Vec::new();
    for s in &found.samples {
        if s.r2.is_none() {
            let name = s.r1.file_name().unwrap_or_default().to_string_lossy();
            out.push(format!("{}: {name} has no matching {} file.", s.sample, found.r2));
        }
    }
    out.extend(same_names(found, "pairs"));
    out
}

/// Two samples can't have one name: their results would land in one folder.
fn same_names(found: &Found, what: &str) -> Vec<String> {
    let mut seen: Vec<(&str, usize)> = Vec::new();
    for s in &found.samples {
        match seen.iter_mut().find(|(n, _)| *n == s.sample) {
            Some((_, c)) => *c += 1,
            None => seen.push((&s.sample, 1)),
        }
    }
    seen.into_iter()
        .filter(|(_, c)| *c > 1)
        .map(|(name, count)| {
            if what == "pairs" {
                format!("Sample name {name} is used by {count} pairs: rename the files or change the pairing.")
            } else {
                format!("Sample name {name} is used by {count} files: rename one of them.")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["s10", "S2", "s1"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["s1", "S2", "s10"]);
    }

    #[test]
    fn markers() {
        assert_eq!(last_marker("A_R1_001.fastq.gz", "_R1"), Some(1));
        assert_eq!(last_marker("A_R10_001.fastq.gz", "_R1"), None);
        assert_eq!(last_marker("A_R1_x_R1.fq", "_R1"), Some(6));
    }

    #[test]
    fn pairs_folder() {
        let d = std::env::temp_dir().join(format!("gwm-reads-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        for n in [
            "S0000066_CTRL_SAL_S29_L001_R1_001.fastq.gz",
            "S0000066_CTRL_SAL_S29_L001_R2_001.fastq.gz",
            "B10_R1.fq.gz",
            "B10_R2.fq.gz",
            "B2_R1.fq.gz",
            "notes.txt",
            "lonely.fastq.gz",
        ] {
            fs::write(d.join(n), b"").unwrap();
        }
        let f = find_samples(&d, None);
        let names: Vec<&str> = f.samples.iter().map(|s| s.sample.as_str()).collect();
        assert_eq!(names, ["B2", "B10", "S0000066_CTRL_SAL_S29_L001"]);
        assert_eq!(f.r1, "_R1");
        assert!(f.samples[0].r2.is_none());
        assert_eq!(f.unpaired, ["lonely.fastq.gz"]);
        assert_eq!(pairing_problems(&f), ["B2: B2_R1.fq.gz has no matching _R2 file."]);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn fasta_folder() {
        let d = std::env::temp_dir().join(format!("gwm-fasta-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        for n in [
            "B10.fasta",
            "b2.fa",
            "C.contigs.fna.gz",
            "C.contigs.fas",
            "notes.txt",
            "reads_R1.fastq.gz",
        ] {
            fs::write(d.join(n), b"").unwrap();
        }
        let f = find_input(&d, ASSEMBLIES, None);
        let names: Vec<&str> = f.samples.iter().map(|s| s.sample.as_str()).collect();
        assert_eq!(names, ["b2", "B10", "C.contigs", "C.contigs"]);
        assert!(f.samples.iter().all(|s| s.r2.is_none()));
        assert_eq!(f.mode, FASTA_MODE);
        assert_eq!(
            input_problems(&f),
            ["Sample name C.contigs is used by 2 files: rename one of them."]
        );
        assert_eq!(first_fasta(&d), Some(d.join("b2.fa")));
        assert_eq!(first_fasta(&d.join("B10.fasta")), Some(d.join("B10.fasta")));
        assert_eq!(first_fasta(&d.join("notes.txt")), None);
        // the same folder as reads: the FASTA files are not samples
        assert_eq!(find_input(&d, "paired_reads", None).samples.len(), 1);
        fs::remove_dir_all(&d).ok();
    }
}
