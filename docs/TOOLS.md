# Writing tools and tool packs

## tool.yml

A tool is a folder `tools/<id>/` with a `tool.yml` and any files it needs.

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

Built-in workflows are read-only. Workflows saved from the UI go to
`~/.ntg/workflows/<id>/v<N>.yml`, one file per version; user workflows can be
deleted (all versions), built-ins can't.

`tool_version: <version>` on a step picks another installed version of its
tool; without it the tool's own version is used. Extra versions are added on
the Tools page (BioContainers tags from quay.io for built-in tools, a commit of
the pack's repository for tool packs) and live in
`~/.ntg/tool_versions/<tool>/<version>/tool.yml`, next to the original. A new
version is `untested: true` until a run that used it succeeds.

## Tool packs

A tool pack brings tools from another git repository. It is a file in
`packs/` (or `ntg/pack.yml` inside the tool's own repository, for install from
a git URL):

```yaml
id: my_pack
name: My pack
repo: https://github.com/owner/repo
commit: <full commit hash>      # what gets installed; update to upgrade
updates: auto                   # what counts as an update (optional, see below)
description: ...
container:
  image: quay.io/...            # a published image, or:
  build:
    environment_file: environment.yml     # conda env in the repo, or
    # channels: [conda-forge, bioconda]
    # dependencies: [blast=2.16.0, python=3.12]
tools:
  - id: ...                     # same fields as tool.yml
    files: [tool/script.py, database]     # paths inside the repo
```

Installing clones the repo at the pinned commit, builds the container from
the conda environment (micromamba) when needed, and copies each tool's files
to `~/.ntg/tools/<pack>/<tool>/`. The pack's repo and commit are recorded with
each tool and shown in the UI.

`updates` says when the Tools page shows a pack as having a newer version:

- `releases`: a release tag newer than the installed one (`v1.2.0` or `1.2`).
  Tags with a suffix, such as `v1.2.0-galaxy1` or `v1.3.0-rc1`, don't count.
- `main`: any commit on the repo's main branch that isn't installed.
- `auto` (default): `releases` if the repo has a release tag, else `main`.

Each computer can change it per pack on the Tools page (saved in
`~/.ntg/pack_updates.json`).

## Importing a Galaxy workflow

**Workflows › Import from Galaxy** (v2 app) reads a workflow file saved from
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
  `command:` has to be written by hand; remove `draft: true` when it is.
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
