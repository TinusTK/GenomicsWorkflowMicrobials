# Tools: how they are described, added and edited

Most people never need this page: **Tools › Add a tool** in the app takes a
GitHub link and its editor writes the `tool.yml` for you (see
[Adding a tool from GitHub](#adding-a-tool-from-github)). This is the format
underneath.

## tool.yml

A tool is a folder with a `tool.yml` and any files it needs. Built-in tools
are in `tools/<id>/`; tools added on a computer are in `~/.gwm/tools/`.

```yaml
id: mytool                 # lowercase letters, digits, _
name: My tool
version: 1.2.3
category: Typing           # groups tools in the UI
description: What it does, in a sentence or two.
homepage: https://...
license: MIT
citation: "..."
container: quay.io/biocontainers/mytool:1.2.3--h1234_0   # pin an exact tag
scope: per_sample          # or all_samples (runs once on every sample's file)
resources: {cpus: 4, memory_gb: 4}
# Optional: let a setting choose the memory (0/blank = Auto = memory_gb, or
# low_memory_gb when the boolean low_memory_setting is on; never more than the
# computer's RAM minus 2 GB, at least 4 GB). See tools/shovill/tool.yml.
#   memory_setting: ram
#   low_memory_setting: low_memory
#   low_memory_gb: 8
requirements:              # used for the hardware warnings
  memory_gb: 2             # error below this
  recommended_memory_gb: 8 # warning below this
  disk_gb_per_sample: 1
  note: Shown next to the step.
databases: [kraken2_standard_8]   # ids from databases/catalog.yml
files: [script.py]         # files/folders in the tool folder the command uses

inputs:
  contigs: {type: fasta, description: Assembled contigs.}
  # types: reads (a pair), fasta, tsv, txt, log, any ...; optional: true allowed

outputs:
  report: {type: tsv, path: "out/{sample}.mytool.tsv", description: ...}
  # files under out/ are published to the results; others (e.g. trimmed/) are
  # only passed to the next steps. {sample} is the sample name.

settings:
  minid:
    label: Minimum identity (%)
    type: number            # integer, number, string, boolean, select (+ options: [...])
    default: 90
    description: Shown under the setting in the UI.
  scheme:
    label: Scheme
    type: string
    default: ""
    auto: {value: "", label: Auto-detect}   # gives the setting an Auto on/off toggle

version_command: mytool --version     # or versions: {name: command, ...}

command: |
  mytool --threads "$CPUS" --minid "$S_minid" "$IN_contigs" > "out/$SAMPLE.mytool.tsv"
```

### What the command can use

| Variable | Meaning |
|---|---|
| `SAMPLE` | sample name (`all` for all-sample steps) |
| `CPUS`, `MEM_GB` | resources given to the step |
| `S_<setting>` | each setting, as text (`true` / `false` for booleans) |
| `IN_<input>` | per-sample input file; reads: `IN_<input>_1`, `IN_<input>_2` |
| `IN_<input>_DIR` | all-sample input: folder holding every sample's file |
| `DB_<database>` | database folder |
| `TOOL_DIR` | folder holding the tool's `files` |

Write results under `out/`. Lines `key: value` written to `.ntg_notes` go into
the run's provenance (e.g. a choice made automatically).

## Workflows

`workflows/<id>/workflow.yml` lists steps; each step names a tool, connects its
inputs to `input` (the raw reads) or `<step>.<output>`, and sets settings. A
list of sources means "the first one that runs", e.g.
`reads: [fastp.reads, input]` uses raw reads when fastp is off.
`extends: <id>` starts from another workflow's steps.

Built-in workflows are read-only. Workflows saved in the app go to
`~/.gwm/workflows/<id>/v<N>.yml`, one file per version; your own workflows can
be deleted (all versions), built-ins can't.

`tool_version: <version>` on a step picks another installed version of its
tool; without it the tool's own version is used. Extra versions are added on
the Tools page (**See other versions**, or **Check for newer versions**: the
BioContainers images of the tool on quay.io) and live in
`~/.gwm/tool_versions/<tool>/<version>/tool.yml`, next to the original.

## Adding a tool from GitHub

**Tools › Add a tool** takes the link to a tool's GitHub page. Nothing on
GitHub is changed; the tool is kept in `~/.gwm/tools/github/<id>/`.

**A repository with its own definition** installs as it is. Put a `tool.yml`
(the format above) in a folder called `gwm/` at the top of the repository;
the files it lists under `files:` are taken from that same folder:

```
gwm/tool.yml
gwm/script.py        # listed as files: [script.py]
```

**Any other repository** becomes a draft. The app fills in what it can find:

- name, description, licence and website, from GitHub;
- the version, from the newest release;
- the container, from BioContainers: it looks for the package named in the
  README (a bioconda badge or a `conda install` line) and then for the
  repository's own name. With none found, the container address is typed in
  the editor (any public image with the tool installed, at one exact tag).

The draft opens in the editor, where the command, inputs and outputs are
filled in. The editor lists the names the command can use, checks the form as
you type, shows the tool's own help text (**Show the tool's help**) and runs
the command once on one sample (**Try on one sample**), listing the files it
made and how each will show on a run's page. A draft can't be run in a
workflow until **Ready to use** is switched on (`draft: true` is then taken
out of its `tool.yml`).

Tools made by a Galaxy import (below) are edited the same way.

If a tool will not install or run, please
[open an issue](https://github.com/TinusTK/GenomicsWorkflowMicrobials/issues/new)
with its GitHub link.

## Importing a Galaxy workflow

**Workflows › Import from Galaxy** reads a workflow file saved from
Galaxy (`.ga`) and makes a workflow here with the same tools, versions,
settings and wiring. It shows what each Galaxy step becomes before anything
is added:

- **In the app**: a tool here at the same version.
- **New version**: a tool here at another version. That version is added beside
  the default one in `~/.gwm/tool_versions/<tool>/<version>/` with the matching
  BioContainers image, and the step pins it with `tool_version`.
- **New tool**: a tool the app does not know. A draft goes in
  `~/.gwm/tools/galaxy_import/<tool>/tool.yml` with its pinned container,
  inputs, outputs and settings. A `.ga` file has no command lines, so the
  command is written in the tool editor (Tools page, open the tool, **Finish
  this tool**).
- **Summary tables**: Galaxy text and collection steps (cut, join, collapse ...)
  that the `summary` tool covers.
- **Left out**: nothing here matches.

A tool is matched through the `galaxy:` block of its `tool.yml`:

```yaml
galaxy:
  tool: iuc/shovill/shovill            # Tool Shed owner/repository/tool id
  inputs: {"library|input1": reads}    # Galaxy input name -> this tool's input
  outputs: {contigs: contigs, shovill_std_log: log}
  settings:                            # this tool's setting <- place in Galaxy's tool_state
    depth: adv.depth
    nocorr: {from: adv.keep_files.nocorr, map: {no_correction: true}, default: false}
    raw: {from: raw_outputs, has: summary}   # true when Galaxy's list holds the value
  any_version: true                    # optional: stands in for every Galaxy version
  step: {publish: summary/shigella}    # optional: added to every imported step
```

`rust/crates/gwm-core/src/galaxy.rs` has the full format; its tests check that
importing the RDH `.ga` files in `reference/` gives the built-in workflow.

## Tool containers built by this repository

Tools with no BioContainers image (Shigella typing, C. diphtheriae typing) get
theirs from `containers/<name>/Dockerfile`, built and published by
`.github/workflows/containers.yml` as `ghcr.io/tinustk/gwm-<name>:<version>`.
A published version is never built over: change the version in the workflow
file and in the tool's `tool.yml` together.

## Summary tables: an input that takes any number of outputs

The built-in **Summary tables** tool (`tools/summary_tables`) shows the tables
of whatever steps are connected to it on a run's Results page, one table per
connected output. Its one input is a `collect` input:

```yaml
inputs:
  tables:
    type: table
    collect: true              # every connected source is used, not just the first that runs
    accepts: [tsv, csv, tab]   # the output types that fit
    optional: true
```

In a workflow, list as many sources as you like:
`inputs: {tables: [mlst.report, abricate.report, staramr.summary]}`. The
command finds one folder per source in `$IN_tables_DIR`, named
`<step>.<output>`, and every sample id of the run in `$SAMPLES` (one per
line). The RDH workflow keeps its own `summary` tool, which makes the tables
exactly as Galaxy does.
