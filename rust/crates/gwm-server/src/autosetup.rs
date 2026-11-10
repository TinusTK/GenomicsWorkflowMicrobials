//! "Set it up for me": for a tool with no tested setup, read the tool's own
//! help text, work out a likely command, try it once on a sample, and list
//! what it made as the tool's outputs. A best guess: it fits tools with the
//! usual options (reads in, a folder or file out), not every tool.

use std::path::Path;

use anyhow::{bail, Result};
use gwm_core::catalog::{dump_yaml, load_yaml};
use gwm_core::Catalog;
use gwm_engine::trial::TrialResult;
use serde_json::json;
use serde_yaml::{Mapping, Value};

use crate::jobs::Reporter;
use crate::tooledit::{help_text, own_tool, run_try, y, TryRequest};

/// One option of a program, from its help text.
#[derive(Debug, Clone, PartialEq)]
struct Opt {
    /// as typed: `--read1`, `-1`
    flags: Vec<String>,
    /// what the help says about it, lowercase
    about: String,
}

impl Opt {
    /// The flag to type: the long one when there is one.
    fn flag(&self) -> &str {
        self.flags
            .iter()
            .find(|f| f.starts_with("--"))
            .unwrap_or(&self.flags[0])
    }

    /// Its names without dashes, lowercase, `_` as `-`.
    fn names(&self) -> Vec<String> {
        self.flags
            .iter()
            .map(|f| f.trim_start_matches('-').to_lowercase().replace('_', "-"))
            .collect()
    }

    fn is(&self, names: &[&str]) -> bool {
        self.names().iter().any(|n| names.contains(&n.as_str()))
    }

    fn says(&self, words: &[&str]) -> bool {
        words.iter().any(|w| self.about.contains(w))
    }
}

/// The options a help text lists: lines that start with a `-flag`, with the
/// lines under them as their description.
fn options(help: &str) -> Vec<Opt> {
    let mut out: Vec<Opt> = Vec::new();
    for line in help.lines() {
        let text = line.trim_start();
        let indent = line.len() - text.len();
        if text.starts_with('-') && indent < 12 {
            // flags come first, the description after two or more spaces
            let (head, about) = match text.find("  ") {
                Some(i) => (&text[..i], text[i..].trim()),
                None => (text, ""),
            };
            let flags: Vec<String> = head
                .split([' ', ',', '=', '|', '['])
                .filter(|w| w.starts_with('-') && w.len() > 1 && w.chars().nth(1) != Some(' '))
                .map(|w| w.trim_end_matches([']', ')', ':']).to_string())
                .filter(|w| {
                    w.trim_start_matches('-')
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                })
                .filter(|w| !w.trim_start_matches('-').is_empty())
                .collect();
            if !flags.is_empty() {
                // the words after the flags on the same line (FILE, DIR) say something too
                out.push(Opt {
                    flags,
                    about: format!("{head} {about}").to_lowercase(),
                });
                continue;
            }
        }
        if let Some(last) = out.last_mut() {
            if indent >= 4 && !text.is_empty() {
                last.about.push(' ');
                last.about.push_str(&text.to_lowercase());
            }
        }
    }
    out
}

/// The sub-command that does the tool's main job, when the program has
/// sub-commands (`tb-profiler profile`).
fn subcommand(help: &str) -> Option<String> {
    const MAIN: [&str; 16] = [
        "profile", "run", "typing", "type", "call", "predict", "analyse", "analyze", "screen", "scan", "search",
        "detect", "classify", "assemble", "query", "all",
    ];
    let mut found: Vec<String> = Vec::new();
    // argparse: {profile,lineage,collate}
    for part in help.split('{').skip(1) {
        if let Some(list) = part.split('}').next() {
            if list.contains(',') && !list.contains(' ') {
                found.extend(list.split(',').map(|s| s.trim().to_lowercase()));
            }
        }
    }
    // a "Commands:" section: one indented word per line, then its description
    let mut in_commands = false;
    for line in help.lines() {
        let low = line.trim().to_lowercase();
        if low.ends_with("commands:") || low == "commands" || low.ends_with("subcommands:") {
            in_commands = true;
            continue;
        }
        if in_commands {
            if line.trim().is_empty() {
                continue;
            }
            if !line.starts_with(' ') {
                in_commands = false;
                continue;
            }
            if let Some(word) = line.split_whitespace().next() {
                if word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    found.push(word.to_lowercase());
                }
            }
        }
    }
    MAIN.iter()
        .find(|m| found.iter().any(|f| f == *m))
        .map(|m| m.to_string())
}

/// A command worked out from a help text.
#[derive(Debug, Default, PartialEq)]
struct Guess {
    command: String,
    /// "reads" or "fasta": what the tool takes in
    input: String,
    /// where the result goes when the command names one file
    output: Option<String>,
    /// what was decided, in plain words
    notes: Vec<String>,
}

fn guess(program: &str, id: &str, help: &str) -> Guess {
    let opts = options(help);
    let find = |names: &[&str]| opts.iter().find(|o| o.is(names));
    let low = help.to_lowercase();
    let mut g = Guess::default();
    let mut parts = vec![program.to_string()];

    let r1 = find(&[
        "read1", "reads1", "r1", "fq1", "fastq1", "fastq-1", "pe1", "forward", "fwd", "in1", "input1", "mate1", "1",
    ]);
    let r2 = find(&[
        "read2", "reads2", "r2", "fq2", "fastq2", "fastq-2", "pe2", "reverse", "rev", "in2", "input2", "mate2", "2",
    ]);
    let fasta = find(&["fasta", "assembly", "contigs", "genome", "fna", "assemblies"]).or_else(|| {
        opts.iter()
            .find(|o| o.is(&["i", "input", "infile"]) && o.says(&["fasta", "assembl", "contig", "genome"]))
    });
    let mut tail = String::new();
    if let (Some(a), Some(b)) = (r1, r2) {
        parts.push(format!("{} \"$IN_reads_1\" {} \"$IN_reads_2\"", a.flag(), b.flag()));
        g.input = "reads".into();
        g.notes.push(format!(
            "It takes the two read files with {} and {}.",
            a.flag(),
            b.flag()
        ));
    } else if let Some(f) = fasta {
        parts.push(format!("{} \"$IN_contigs\"", f.flag()));
        g.input = "fasta".into();
        g.notes
            .push(format!("It takes an assembly (contigs) with {}.", f.flag()));
    } else if low.contains("fastq") || low.contains("paired") {
        tail = " \"$IN_reads_1\" \"$IN_reads_2\"".into();
        g.input = "reads".into();
        g.notes
            .push("No option for the read files was found, so they go at the end of the command.".into());
    } else {
        tail = " \"$IN_contigs\"".into();
        g.input = "fasta".into();
        g.notes
            .push("No option for its input was found, so an assembly (contigs) goes at the end of the command.".into());
    }

    let threads = find(&[
        "threads",
        "thread",
        "cpus",
        "cpu",
        "cores",
        "nthreads",
        "num-threads",
        "numthreads",
        "nproc",
        "ncpus",
    ])
    .or_else(|| {
        opts.iter()
            .find(|o| o.is(&["t", "p", "j"]) && o.says(&["thread", "cpu", "core"]))
    });
    if let Some(t) = threads {
        parts.push(format!("{} \"$CPUS\"", t.flag()));
    }

    let folder_words = ["dir", "folder"];
    let outdir = find(&[
        "outdir",
        "out-dir",
        "output-dir",
        "outputdir",
        "dir",
        "output-directory",
        "out-folder",
    ])
    .or_else(|| {
        opts.iter()
            .find(|o| o.is(&["o", "out", "output"]) && o.says(&folder_words))
    });
    let outfile = opts
        .iter()
        .find(|o| o.is(&["o", "out", "output", "outfile", "output-file"]) && !o.says(&folder_words));
    let prefix = find(&["prefix", "sample-name", "sample-id"]).or_else(|| {
        opts.iter()
            .find(|o| o.is(&["p", "n", "name", "sample", "id"]) && o.says(&["prefix", "sample name", "sample id"]))
    });
    match (outdir, prefix, outfile) {
        (Some(d), p, _) => {
            if let Some(p) = p {
                parts.push(format!("{} \"$SAMPLE\"", p.flag()));
            }
            parts.push(format!("{} out", d.flag()));
            g.notes.push(format!(
                "Its results go into the out folder with {}{}.",
                d.flag(),
                p.map(|p| format!(", named after the sample with {}", p.flag()))
                    .unwrap_or_default()
            ));
        }
        (None, Some(p), _) => {
            parts.push(format!("{} \"out/$SAMPLE\"", p.flag()));
            g.notes.push(format!(
                "Its results are named after the sample, in the out folder, with {}.",
                p.flag()
            ));
        }
        (None, None, Some(o)) => {
            parts.push(format!("{} \"out/$SAMPLE.{id}.tsv\"", o.flag()));
            g.output = Some(format!("out/{{sample}}.{id}.tsv"));
            g.notes
                .push(format!("Its result is written to one file with {}.", o.flag()));
        }
        (None, None, None) => {
            tail.push_str(&format!(" > \"out/$SAMPLE.{id}.tsv\""));
            g.output = Some(format!("out/{{sample}}.{id}.tsv"));
            g.notes
                .push("No option for where results go was found, so what it prints is saved as the result.".into());
        }
    }
    g.command = format!("{}{tail}", parts.join(" "));
    g
}

/// The result files worth showing, as outputs: small text, table and page
/// files under out/, with the sample's name written `{sample}`.
fn outputs_from(done: &TrialResult) -> Mapping {
    let mut out = Mapping::new();
    for f in &done.files {
        let name = f.path.rsplit('/').next().unwrap_or(&f.path);
        let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
        let kind = match ext.as_str() {
            "tsv" | "tab" => "tsv",
            "csv" => "csv",
            "txt" => "txt",
            "json" => "json",
            "html" | "htm" => "html",
            _ => continue,
        };
        if !f.path.starts_with("out/") || f.size == 0 || f.size > 50_000_000 || out.len() >= 8 {
            continue;
        }
        let path = if done.sample != "all" {
            f.path.replace(&done.sample, "{sample}")
        } else {
            f.path.clone()
        };
        let stem = Path::new(&name.replace(&done.sample, ""))
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut key: String = stem
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string();
        while key.contains("__") {
            key = key.replace("__", "_");
        }
        if !key.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            key = format!("report_{key}").trim_end_matches('_').to_string();
        }
        if kind != "tsv" && kind != "csv" && !key.ends_with(kind) {
            key = format!("{key}_{kind}");
        }
        while out.contains_key(key.as_str()) {
            key.push_str("_2");
        }
        let mut e = Mapping::new();
        e.insert(y("type"), y(kind));
        e.insert(y("path"), y(path));
        out.insert(y(key), Value::Mapping(e));
    }
    out
}

/// Set a draft tool up: its command from its help text, then (with a folder of
/// reads, or of FASTA files for a tool that takes an assembly) a try-out, its outputs from what the try-out made, and Ready to use
/// when that worked.
pub fn set_up(cat: &Catalog, id: &str, reads_folder: &str, r: &Reporter) -> Result<()> {
    let tool = own_tool(cat, id)?;
    if tool.container.trim().is_empty() {
        bail!("this tool has no container yet: fill in its container address first (More options)");
    }
    let base = tool
        .extra
        .get("program")
        .and_then(Value::as_str)
        .map(|p| p.split_whitespace().next().unwrap_or(p).to_string())
        .unwrap_or_else(|| tool.id.replace('_', "-"));
    r.log("Reading the tool's help (getting its container first if it is not here yet)");
    let mut help = help_text(cat, &tool, &base)?;
    if help.trim().is_empty() {
        bail!("{base} printed no help: check the program's name and the container (More options)");
    }
    let mut program = base.clone();
    if let Some(sub) = subcommand(&help) {
        let deeper = help_text(cat, &tool, &format!("{base} {sub}"))?;
        if !options(&deeper).is_empty() {
            r.log(format!("{base} has several jobs: using \"{sub}\""));
            program = format!("{base} {sub}");
            help = deeper;
        }
    }
    let g = guess(&program, &tool.id, &help);
    for n in &g.notes {
        r.log(n.clone());
    }

    let file = tool.dir.join("tool.yml");
    let Value::Mapping(mut raw) = load_yaml(&file)? else {
        bail!("{}: not a tool definition", file.display())
    };
    raw.insert(y("program"), y(&program));
    raw.insert(y("command"), y(&g.command));
    let mut inputs = Mapping::new();
    inputs.insert(
        y(if g.input == "reads" { "reads" } else { "contigs" }),
        serde_yaml::from_str(if g.input == "reads" {
            "{type: reads, description: The paired reads of one sample.}"
        } else {
            "{type: fasta, description: The assembly (contigs) of one sample.}"
        })?,
    );
    raw.insert(y("inputs"), Value::Mapping(inputs));
    let mut outputs = Mapping::new();
    if let Some(path) = &g.output {
        outputs.insert(
            y("report"),
            serde_yaml::from_str(&format!("{{type: tsv, path: \"{path}\"}}"))?,
        );
    }
    raw.insert(y("outputs"), Value::Mapping(outputs));
    raw.insert(y("draft"), Value::Bool(true));
    dump_yaml(&raw, &file)?;

    let mut result = json!({"command": g.command, "notes": g.notes, "tried": false, "ready": false});
    let fasta = g.input != "reads";
    let folder = gwm_core::paths::native_path(reads_folder.trim());
    // what to try the command on: the folder given, else the repository's own test file
    let request = if reads_folder.trim().is_empty() {
        match crate::selftest::test_input(cat, &own_tool(cat, id)?, r) {
            Some((req, what)) => {
                r.log(format!(
                    "No folder was given, so the command is tried on the repository's own test file ({what})."
                ));
                Some(req)
            }
            None => {
                r.log(if fasta {
                    "This tool works on an assembly. No folder of FASTA files was given and its repository has no test file to use, so the command was not tried: choose a folder and press Set it up for me again."
                } else {
                    "No folder of reads was given and the repository has no test reads to use, so the command was not tried."
                });
                None
            }
        }
    } else if fasta && gwm_core::reads::first_fasta(&folder).is_none() {
        r.log("This tool works on an assembly, and the folder has no FASTA file (.fasta, .fa, .fna, .fas, .ffn): choose a folder of FASTA files and press Set it up for me again.");
        None
    } else {
        Some(TryRequest {
            reads_folder: reads_folder.to_string(),
            ..Default::default()
        })
    };
    if let Some(request) = request {
        r.log("Trying the command on one sample");
        let tool = own_tool(cat, id)?;
        let done = run_try(cat, &tool, &request)?;
        result["tried"] = json!(true);
        if done.exit == 0 {
            let found = outputs_from(&done);
            if found.is_empty() {
                r.log("The command ran, but made no result files in the out folder that the app can show.");
            } else {
                r.log(format!(
                    "It worked and made {} result file{}: the tool is ready to use.",
                    found.len(),
                    if found.len() == 1 { "" } else { "s" }
                ));
                raw.insert(y("outputs"), Value::Mapping(found));
                raw.remove("draft");
                dump_yaml(&raw, &file)?;
                result["ready"] = json!(true);
            }
        } else {
            r.log("The command stopped with an error: it needs changing by hand. What it printed is below.");
        }
        result["trial"] = serde_json::to_value(&done)?;
    }
    r.result(result);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TB: &str = "\
Usage: tb-profiler profile [-h] [--read1 READ1] [--read2 READ2]

Input Options:
  --read1, -1 READ1     First read file (default: None)
  --read2, -2 READ2     Second read file (default: None)
  --fasta, -f FASTA     Fasta file (default: None)

Output Options:
  --prefix, -p PREFIX   Sample prefix for all results generated (default:
                        tbprofiler)
  --csv                 Add CSV output (default: False)
  --dir, -d DIR         Storage directory (default: .)

Other:
  --threads, -t THREADS
                        Threads to use (default: 1)
";

    #[test]
    fn options_and_their_descriptions() {
        let o = options(TB);
        assert_eq!(o[0].flags, ["--read1", "-1"]);
        assert!(o
            .iter()
            .any(|o| o.flags == ["--prefix", "-p"] && o.about.contains("tbprofiler)")));
        assert!(o
            .iter()
            .any(|o| o.flags == ["--threads", "-t"] && o.about.contains("threads to use")));
    }

    #[test]
    fn a_tool_with_read_and_folder_options() {
        let g = guess("tb-profiler profile", "tb_profiler", TB);
        assert_eq!(
            g.command,
            "tb-profiler profile --read1 \"$IN_reads_1\" --read2 \"$IN_reads_2\" --threads \"$CPUS\" --prefix \"$SAMPLE\" --dir out"
        );
        assert_eq!(g.input, "reads");
        assert_eq!(g.output, None);
    }

    #[test]
    fn a_tool_that_prints_its_result() {
        let help = "SYNOPSIS\n  Automatic MLST calling from assembled contigs\nUSAGE\n  % mlst [options] <contigs.fasta>\nOPTIONS\n  --threads [N]     Number of BLAST threads (default '1')\n  --scheme [X]      Don't autodetect, force this scheme\n";
        let g = guess("mlst", "mlst", help);
        assert_eq!(
            g.command,
            "mlst --threads \"$CPUS\" \"$IN_contigs\" > \"out/$SAMPLE.mlst.tsv\""
        );
        assert_eq!(g.input, "fasta");
        assert_eq!(g.output.as_deref(), Some("out/{sample}.mlst.tsv"));
    }

    #[test]
    fn the_main_sub_command() {
        let help = "Usage: tb-profiler [--version]\n   {profile,lineage,spoligotype,collate}\n";
        assert_eq!(subcommand(help).as_deref(), Some("profile"));
        assert_eq!(
            subcommand("Commands:\n  index   Build an index\n  run     Run the typing\n").as_deref(),
            Some("run")
        );
        assert_eq!(subcommand("usage: fastp -i in.fq -o out.fq"), None);
    }

    #[test]
    fn every_recipe_is_a_ready_tool() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../recipes");
        let mut n = 0;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let tool: gwm_core::Tool = serde_yaml::from_value(load_yaml(&e.path()).unwrap()).unwrap();
            assert!(
                !tool.command.trim().is_empty() && !tool.container.is_empty(),
                "{}",
                tool.id
            );
            assert!(
                !tool.outputs.is_empty() && tool.extra.get("draft").is_none(),
                "{}",
                tool.id
            );
            n += 1;
        }
        assert!(n > 0);
    }
}
