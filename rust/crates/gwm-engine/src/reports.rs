//! A tool's report as table files. Many tools write a report rather than a
//! table: a text file in titled sections ("Summary", "Lineage report" ...) or
//! a JSON file. At the end of such a tool's step the report is also written as
//! tables (`out/tables/<sample>.<section>.tsv`), which the tool offers as its
//! `tables` output, so they can be connected to the Summary tables tool like
//! any other table. The tool's own files are left as they are.
//!
//! - a section of `name: value` lines becomes a table of one row, the names
//!   as headings;
//! - a section of rows (tabs between cells, commas in a .csv) becomes a table;
//! - a JSON file gives its plain values as one row and each list of records
//!   as a table. It is used only when no text report gave a table.
//!
//! The Run page reads reports the same way for runs made before this
//! (`rust/gui/src/lib/results.ts`): keep the two in step.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use indexmap::IndexMap;
use serde_json::Value;

/// One section of a report as a table.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: String,
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// A line of only ---- or ==== (under a section's title).
fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && t.chars().all(|c| "-=~_*".contains(c))
}

/// `name: value`
fn pair(line: &str) -> Option<(String, String)> {
    let (name, value) = line.split_once(':')?;
    let name = name.trim();
    if name.is_empty() || name.len() > 60 || name.contains('\t') {
        return None;
    }
    // "12:30" or "http://x" is a value, not a name
    if !value.is_empty() && !value.starts_with([' ', '\t']) {
        return None;
    }
    Some((name.to_string(), value.trim().to_string()))
}

fn numeric(s: &str) -> bool {
    !s.is_empty() && s.parse::<f64>().is_ok()
}

fn table_of(title: &str, body: &[&str], sep: char) -> Option<Section> {
    let lines: Vec<&str> = body.iter().copied().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return None;
    }
    let split: Vec<Vec<String>> = lines
        .iter()
        .map(|l| l.split(sep).map(|c| c.trim().to_string()).collect())
        .collect();
    let width = split[0].len();
    let title = if title.is_empty() { "Summary" } else { title }.to_string();

    // name: value on every line
    let pairs: Vec<Option<(String, String)>> = lines.iter().map(|l| pair(l)).collect();
    if pairs.iter().all(Option::is_some) {
        let (header, row): (Vec<String>, Vec<String>) = pairs.into_iter().flatten().unzip();
        return Some(Section {
            title,
            header,
            rows: vec![row],
        });
    }
    // two cells a line ("ID,DRR1"): names and values too, unless it reads as a table of numbers under a heading
    let two = width == 2 && split.iter().all(|r| r.len() == 2);
    let headed = two && split.len() >= 3 && !numeric(&split[0][1]) && split[1..].iter().all(|r| numeric(&r[1]));
    if two && split.len() <= 40 && !headed {
        return Some(Section {
            title,
            header: split.iter().map(|r| r[0].clone()).collect(),
            rows: vec![split.iter().map(|r| r[1].clone()).collect()],
        });
    }
    let fitting = split.iter().filter(|r| r.len() == width).count();
    if width >= 2 && split.len() >= 2 && fitting * 5 >= split.len() * 4 {
        return Some(Section {
            title,
            header: split[0].clone(),
            rows: split[1..].to_vec(),
        });
    }
    None
}

/// A text report in sections: a title line with a line of ---- or ==== under it.
pub fn text_sections(text: &str, sep: char) -> Vec<Section> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut title = "";
    let mut body: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if !line.trim().is_empty() && !is_rule(line) && lines.get(i + 1).is_some_and(|n| is_rule(n)) {
            out.extend(table_of(title, &body, sep));
            body.clear();
            title = line.trim();
            i += 2;
            continue;
        }
        if !is_rule(line) {
            body.push(line);
        }
        i += 1;
    }
    out.extend(table_of(title, &body, sep));
    out
}

fn cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(cell).collect::<Vec<_>>().join("; "),
        other => other.to_string(),
    }
}

fn plain(v: &Value) -> bool {
    !v.is_object() && !v.is_array()
}

fn json_visit(obj: &serde_json::Map<String, Value>, title: &str, depth: usize, out: &mut Vec<Section>) {
    let scalars: Vec<(&String, &Value)> = obj.iter().filter(|(_, v)| plain(v)).collect();
    if !scalars.is_empty() {
        out.push(Section {
            title: title.to_string(),
            header: scalars.iter().map(|(k, _)| (*k).clone()).collect(),
            rows: vec![scalars.iter().map(|(_, v)| cell(v)).collect()],
        });
    }
    for (k, v) in obj {
        match v {
            Value::Array(a) if !a.is_empty() && a.iter().all(Value::is_object) => {
                let mut header: Vec<String> = Vec::new();
                for rec in a.iter().filter_map(Value::as_object) {
                    for (c, v) in rec {
                        if plain(v) && !header.contains(c) {
                            header.push(c.clone());
                        }
                    }
                }
                if !header.is_empty() {
                    out.push(Section {
                        title: k.clone(),
                        rows: a
                            .iter()
                            .filter_map(Value::as_object)
                            .take(5000)
                            .map(|rec| {
                                header
                                    .iter()
                                    .map(|h| rec.get(h).map(cell).unwrap_or_default())
                                    .collect()
                            })
                            .collect(),
                        header,
                    });
                }
            }
            Value::Object(o) if depth < 1 => json_visit(o, k, depth + 1, out),
            _ => {}
        }
    }
}

/// A JSON report: its plain values as one row, each list of records as a table.
pub fn json_sections(text: &str) -> Vec<Section> {
    let mut out = Vec::new();
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(o)) => json_visit(&o, "Summary", 0, &mut out),
        Ok(Value::Array(a)) => {
            let mut o = serde_json::Map::new();
            o.insert("rows".into(), Value::Array(a));
            json_visit(&o, "Summary", 1, &mut out);
        }
        _ => {}
    }
    out
}

/// A report file's sections; none when it is not that kind of file.
pub fn sections(path: &Path, text: &str) -> Vec<Section> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "json" => json_sections(text),
        "csv" => text_sections(text, ','),
        "txt" | "tsv" | "tab" | "log" => text_sections(text, '\t'),
        _ => Vec::new(),
    }
}

/// A section's title as part of a file name: `Lineage report` -> `lineage_report`.
fn slug(title: &str) -> String {
    let mut s: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "table".into()
    } else {
        s
    }
}

fn clean(c: &str) -> String {
    c.replace(['\t', '\n', '\r'], " ")
}

/// Write the tables of a finished task's reports into `<dir>/out/tables/`.
/// `outputs`: the task's files by output name; `report_outputs`: the outputs
/// that are reports; `tag`: the sample ("all" for an all-sample step). Text
/// reports first; a JSON one only when no text report gave a table.
pub fn write_tables(
    dir: &Path,
    outputs: &IndexMap<String, Vec<PathBuf>>,
    report_outputs: &[String],
    tag: &str,
) -> Result<usize> {
    let files: Vec<&PathBuf> = report_outputs.iter().filter_map(|o| outputs.get(o)).flatten().collect();
    let is_json = |f: &PathBuf| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let mut found: Vec<Section> = Vec::new();
    for json in [false, true] {
        for f in files.iter().filter(|f| is_json(f) == json) {
            // a report, not a data file
            if fs::metadata(f).map(|m| m.len()).unwrap_or(0) > 5_000_000 {
                continue;
            }
            let Ok(text) = fs::read_to_string(f) else { continue };
            found.extend(sections(f, &text));
        }
        if !found.is_empty() {
            break;
        }
    }
    if found.is_empty() {
        return Ok(0);
    }
    let tables = dir.join("out").join("tables");
    fs::create_dir_all(&tables)?;
    let mut used: Vec<String> = Vec::new();
    for s in &found {
        let mut name = slug(&s.title);
        while used.contains(&name) {
            name.push_str("_2");
        }
        used.push(name.clone());
        let file = if tag == "all" {
            format!("{name}.tsv")
        } else {
            format!("{tag}.{name}.tsv")
        };
        let width = s.header.len();
        let mut text = s.header.iter().map(|c| clean(c)).collect::<Vec<_>>().join("\t");
        text.push('\n');
        for row in &s.rows {
            let mut cells: Vec<String> = row.iter().map(|c| clean(c)).collect();
            cells.resize(width.max(cells.len()), String::new());
            text.push_str(&cells.join("\t"));
            text.push('\n');
        }
        fs::write(tables.join(file), text)?;
    }
    Ok(found.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "\nTBProfiler report\n=================\n\nThe following report has been generated by TBProfiler.\n\nSummary\n-------\nID: S1\nStrain: lineage2.2.1.1\nDrug-resistance: Susceptible\nMedian Depth: 53.5\n\n\nLineage report\n--------------\nLineage\tFraction\tFamily\tRd\nlineage2\t100.000\tEast-Asian\tRD105\nlineage2.2\t100.000\tEast-Asian (Beijing)\tRD105;RD207\n\nNotes\n-----\n\n";

    #[test]
    fn a_text_report_in_sections() {
        let s = text_sections(REPORT, '\t');
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!(s[0].title, "Summary");
        assert_eq!(s[0].header, ["ID", "Strain", "Drug-resistance", "Median Depth"]);
        assert_eq!(s[0].rows, [["S1", "lineage2.2.1.1", "Susceptible", "53.5"]]);
        assert_eq!(s[1].title, "Lineage report");
        assert_eq!(s[1].header, ["Lineage", "Fraction", "Family", "Rd"]);
        assert_eq!(s[1].rows.len(), 2);
    }

    #[test]
    fn a_json_report() {
        let s = json_sections(
            r#"{"id":"S1","drtype":"Susceptible","qc":{"depth":53.5},"lineage":[{"lineage":"lineage2","fraction":100},{"lineage":"lineage2.2","fraction":100,"rd":"RD105"}],"deep":{"a":{"b":1}}}"#,
        );
        let titles: Vec<&str> = s.iter().map(|x| x.title.as_str()).collect();
        assert_eq!(titles, ["Summary", "qc", "lineage"]);
        assert_eq!(s[0].header, ["id", "drtype"]);
        assert_eq!(s[2].header, ["lineage", "fraction", "rd"]);
        assert_eq!(s[2].rows[0], ["lineage2", "100", ""]);
    }

    #[test]
    fn plain_text_is_not_a_report() {
        assert!(text_sections("This run finished at 12:30.\nAll good.\n", '\t').is_empty());
        assert!(json_sections("not json").is_empty());
        // lines of one word, three or more of them, and a heading with nothing under it
        assert!(text_sections("Notes\n-----\nalpha\nbeta\ngamma\n\nEmpty\n-----\n", '\t').is_empty());
        assert!(text_sections("a\tb\nonly\n1\n", '\t').is_empty());
    }

    #[test]
    fn tables_written_beside_the_report() {
        let dir = std::env::temp_dir().join(format!("gwm-reports-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("out/results")).unwrap();
        let txt = dir.join("out/results/S1.results.txt");
        let json = dir.join("out/results/S1.results.json");
        fs::write(&txt, REPORT).unwrap();
        fs::write(&json, r#"{"id":"S1"}"#).unwrap();
        let outputs = IndexMap::from([
            ("report".to_string(), vec![txt]),
            ("report_json".to_string(), vec![json]),
        ]);
        let n = write_tables(&dir, &outputs, &["report".into(), "report_json".into()], "S1").unwrap();
        assert_eq!(n, 2); // the text report's tables; the JSON is not needed
        let lineage = fs::read_to_string(dir.join("out/tables/S1.lineage_report.tsv")).unwrap();
        assert!(lineage.starts_with("Lineage\tFraction\tFamily\tRd\nlineage2\t100.000\t"));
        assert_eq!(
            fs::read_to_string(dir.join("out/tables/S1.summary.tsv")).unwrap(),
            "ID\tStrain\tDrug-resistance\tMedian Depth\nS1\tlineage2.2.1.1\tSusceptible\t53.5\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
