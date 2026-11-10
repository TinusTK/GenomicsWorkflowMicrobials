#!/usr/bin/env python3
"""Summary tables: one table per connected output.

--in-dir holds one folder per connected output, named <step>.<output>, with the
files that output made: one per sample (named <sample>...) or one for the
whole run. Per-sample files of the same kind become one table with every
sample's rows and one heading line; a "Sample" column is put in front unless
the rows already start with the sample. Other files are written as they are.

Output: <out-dir>/<step>.<output>.tsv (and <step>.<output>.<kind>.tsv when an
output makes more than one kind of table). Standard library only.
"""
import argparse
import csv
import os
import re
import sys
from pathlib import Path

ABRICATE = ["#FILE", "SEQUENCE", "START", "END", "STRAND", "GENE", "COVERAGE", "COVERAGE_MAP", "GAPS",
            "%COVERAGE", "%IDENTITY", "DATABASE", "ACCESSION", "PRODUCT", "RESISTANCE"]


def headings(name, first):
    """Column headings for the files that have no heading line of their own, else None."""
    if name.endswith(".mlst.tsv"):
        return ["Sample", "Scheme", "ST"] + [f"Allele {i + 1}" for i in range(max(0, len(first) - 3))]
    if name.endswith(".emmtyper.tsv"):
        return ["Sample", "Clusters", "emm type", "Other emm-like alleles", "emm cluster"]
    if name.endswith(".abricate.tsv") and first and first[0] != "#FILE":
        return ABRICATE
    return None


def read_table(path):
    """(header, rows) of a tsv/csv file; None when it is not a text table."""
    try:
        text = path.read_text(encoding="utf-8-sig")
    except (UnicodeDecodeError, OSError):
        return None
    lines = [line for line in text.splitlines() if line.strip()]
    if not lines:
        return [], []
    sep = "\t"
    if path.suffix.lower() == ".csv":
        sep = ";" if lines[0].count(";") > lines[0].count(",") else ","
    rows = list(csv.reader(lines, delimiter=sep)) if sep != "\t" else [line.split("\t") for line in lines]
    known = headings(path.name, rows[0])
    return (known, rows) if known else (rows[0], rows[1:])


def sample_of(name, samples):
    """The sample a file belongs to: the longest sample id its name starts with."""
    hits = [s for s in samples if name.startswith(s)]
    return max(hits, key=len) if hits else None


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--in-dir", required=True)
    ap.add_argument("--out-dir", required=True)
    args = ap.parse_args(argv)
    samples = [s for s in os.environ.get("SAMPLES", "").splitlines() if s]
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    written = 0
    for folder in sorted(p for p in Path(args.in_dir).iterdir() if p.is_dir()):
        # the kinds of table in this output: file name without its sample
        kinds = {}
        for f in sorted(folder.iterdir()):
            table = read_table(f) if f.is_file() else None
            if table is None:
                print(f"{folder.name}: {f.name} is not a table, left out", file=sys.stderr)
                continue
            sample = sample_of(f.name, samples)
            kind = f.name[len(sample):].lstrip("._-") if sample else f.name
            kinds.setdefault(kind, []).append((sample, table))
        for kind, parts in kinds.items():
            header, rows, named = [], [], None
            for sample, (head, body) in parts:
                if not head and not body:
                    continue
                if named is None:  # a Sample column, unless the rows already start with the sample
                    named = sample is None or (bool(body) and sample in body[0][0])
                if not header:
                    header = head if named else ["Sample"] + head
                rows += body if named else [[sample] + r for r in body]
            name = folder.name
            if len(kinds) > 1:
                name += "." + re.sub(r"\.(tsv|csv|tab|txt)$", "", kind)
            with open(out_dir / f"{name}.tsv", "w", newline="") as fh:
                w = csv.writer(fh, delimiter="\t", lineterminator="\n", quoting=csv.QUOTE_NONE, escapechar="\\")
                if header:
                    w.writerow(header)
                w.writerows(rows)
            written += 1
    print(f"{written} table(s) written")


if __name__ == "__main__":
    main()
