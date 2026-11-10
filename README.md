# Genomics Workflow Microbials

A desktop app for typing bacterial isolates from paired Illumina reads. It
runs the Galaxy Australia workflow **"Genomic Assembly and analysis - RDH
version 6"** on your own computer, without Galaxy, and lets you build your own
workflows from the same tools or from tools you add.

- **One installer** for Windows, Mac and Linux. No Nextflow, Java or Python to set up.
- **Every tool in its own container at a fixed version**, so results are repeatable.
- **Every run in one folder** with its results and a record of exactly what ran.
- **Add tools from GitHub** and **import workflows from Galaxy**.

> **Version 2.** Version 1 (the browser app started with `./ntg`) has ended;
> its saved workflows, databases and results are kept.

## Install

Download the installer for your computer from
[Releases](https://github.com/TinusTK/GenomicsWorkflowMicrobials/releases/latest)
and run it.

| Computer | File | Notes |
|---|---|---|
| Windows 10/11 | `..._x64-setup.exe` | Makes a Start menu entry and a desktop shortcut. |
| Mac (Intel or Apple silicon) | `..._universal.dmg` | Drag the app to Applications. |
| Linux | `..._amd64.AppImage` or `..._amd64.deb` | |

The installers are not signed yet, so the first start shows a warning:

- **Windows:** "Windows protected your PC": choose **More info**, then **Run anyway**.
- **Mac:** right-click the app and choose **Open**, then **Open** again.

### What the app needs

The tools run in containers, which need a container engine. The app's Setup
screen and its **Settings** page say what is missing.

- **Windows:** the Linux side of Windows (WSL) with Ubuntu, and Docker inside
  it. The app has buttons for both: **Install WSL** (asks for administrator
  rights and a restart) and **Set up Docker**. Docker Desktop is not needed.
- **Mac:** [Colima](https://github.com/abiosoft/colima)
  (`brew install colima docker`, then `colima start`). The app has not been
  tried on a Mac yet: please [report](#help-and-reporting-problems) what you find.
- **Linux:** Docker Engine (`sudo apt install docker.io`) or Apptainer.

About 10 GB of disk for the reference databases, plus room for runs. 16 GB of
memory or more is best; the app warns before a run about steps that need more
than the computer has.

### Opening and closing

Open it from the desktop shortcut or the Start menu. On Windows and Mac,
closing the window keeps the app in the system tray (the small icons next to
the clock), so it opens again at once. To close it fully, right-click its tray
icon and choose **Quit**. Runs that are going carry on either way, and show
again when you open the app.

### Updating

The app looks for a newer version each time it opens and tells you when one
is out. Press **Update now** (in that message, or in **Settings**): the app
downloads the new version for your computer, installs it over the old one and
opens again. Your workflows, tools, databases and runs are kept, and a run in
progress carries on. Nothing needs uninstalling.

Versions up to 2.0.1 can't do this yet: install 2.0.2 or newer once from the
[download page](https://github.com/TinusTK/GenomicsWorkflowMicrobials/releases/latest)
(run the installer over the old version), and the app updates itself from
then on. Every update is checked against the app's own signature before it is
installed.

## What the built-in workflow does

| Step | Tool | Version |
|---|---|---|
| Read preprocessing | fastp | 0.24.0 |
| Genome assembly | Shovill (SPAdes) | 1.4.2 |
| Species identification | Kraken2 + Bracken | 2.1.3 / 3.0 |
| Serovar | SISTR | 1.1.1 |
| MLST | mlst | 2.22.0 |
| AMR and plasmids | staramr | 0.11.0 |
| emm typing | emmtyper | 0.2.0 |
| Resistance / virulence genes | ABRicate (NCBI, VFDB) | 1.4.0 |
| AMR (Australian reporting) | abriTAMR | 1.3.0 |
| Shigella typing | ShigaPass + Mykrobe | 1.0.2 |
| *C. diphtheriae* tox and MLST typing | built-in | 1.0.0 |
| Completeness / contamination | CheckM | 1.2.3 |
| Assembly QC | QUAST | 5.3.0 |
| Summary tables | built-in | Galaxy's output tables, made the same way |

Settings match the Galaxy workflow (the original file is in
[`reference/`](reference/)). Where the app has to differ from Galaxy, it says
so next to the setting:

- **Kraken2 database:** Standard-8 (2026-06) instead of Galaxy's full Standard
  (2020) with 2021 Bracken files, so species abundances can differ a little.
  It needs about 8 GB of memory; switch the Kraken2 step off on a computer
  that has less. The Databases page also offers Standard-16 (16 GB of memory)
  and the full Standard database (about 100 GB of memory, for servers):
  install one, then pick it in the Kraken2 and Bracken steps (Database).
- **CheckM tree:** "Auto" uses the reduced tree on computers with less than
  40 GB of memory (Galaxy used the full tree).

## Using it

- **New run:** pick a workflow and a folder of reads, check the sample pairs,
  switch steps off for this run if you like, read the warnings for this
  computer, and start.
- **Runs:** live progress drawn as the pipeline, one results table per tool,
  every result file, and what exactly ran. A stopped or failed run carries on
  from where it was.
- **Workflows:** the built-in workflow, and your own. The builder lets you add
  steps, connect them, change settings and save; every save is a new version.
  **Import from Galaxy** turns a workflow file saved from Galaxy (`.ga`) into
  a workflow here, with the same tools, versions and settings.
- **Tools:** every tool with its version and container. **Add a tool** brings
  one in from its GitHub page; **Check for newer versions** and **See other
  versions** add another version of a tool beside the one you have.
- **Databases:** download the reference databases (a broken download carries
  on where it stopped).
- **Settings:** this computer's readiness, where things are kept, updates and
  how to report a problem.

**Reads:** a folder of `<sample>_R1….gz` / `<sample>_R2….gz` files. The app
pairs the files automatically (`_R1/_R2`, `.R1/.R2`, `_1/_2` or `.1/.2`) or
with your own markers, and shows every pair before you start.

## Where things are kept

Each run is one folder in **Documents › Genomics Workflow Microbials**, named
with the date and the run's name. Delete the folder to delete the run.

```
2026-11-10 11-21 My run/
  Results/
    samples/<sample>/<step>/   each tool's output for each sample
    combined/<step>/           tools that run on all samples at once
    summary/                   Galaxy's output tables
    pipeline_info/             provenance.json, tool_versions.yml, trace.tsv
  Run details/                 the workflow, the plan, the samples and the tools exactly as they ran
  README.txt
```

Your own workflows, the tools you added, extra tool versions and the databases
are in `~/.gwm` (on Windows, on the Linux side). **Settings** shows each
folder and opens it.

### Removing the app completely

Uninstalling removes only the program. To take away the rest first, open
**Settings › Remove everything**, press **Show what can be removed** and tick
what should go. Each item shows how much disk it uses:

- **Tool containers:** the downloaded containers the tools run in.
- **App data:** your workflows, added tools, databases and working files.
- **Docker** and **Ubuntu** (Windows only): Docker inside the Linux side of
  Windows, or that whole Ubuntu with everything kept in it.

Nothing is ticked to start with, and you confirm once before anything is
removed. It can't be undone. Your runs in Documents are never touched, and
nothing is removed while a run is going. Then uninstall the app the usual way.

## Adding tools

The app is meant to work with any tool that has a GitHub page.

**Tools › Add a tool**, and paste the tool's GitHub link. Then one of two
things happens:

- **The tool has a tested setup** (the list is in [`recipes/`](recipes/),
  TBProfiler so far): it is ready to use at once.
- **It has none yet:** the app reads the tool's name, licence and newest
  release, finds its container on [BioContainers](https://biocontainers.pro)
  and opens the editor. Choose a folder of reads and press **Set it up for
  me**: the app reads the tool's own help, writes the command, tries it on
  one sample and lists the result files. When that works, the tool is ready.

The automatic setup fits tools with the usual options (reads in, a folder or
file out), not every tool. When it doesn't fit, the command is yours to change
in the editor, which shows the names to use, the tool's help and a try-out on
one sample. Or [ask for a tested setup](#help-and-reporting-problems): once it
is added here, every copy of the app picks it up.

Everything is kept on your computer; nothing is changed on GitHub. See
[docs/TOOLS.md](docs/TOOLS.md) for the details.

If a tool has no ready-made container, the app builds one on your computer
from the tool's repository (several minutes, once; needs Docker). And
whatever a tool writes, its results show as tables: a text or JSON report is
read into tables with headings, every sample together (this can be switched
off per tool on the Tools page).

## Help and reporting problems

If a tool will not install or run, or anything else goes wrong:

- [Open an issue](https://github.com/TinusTK/GenomicsWorkflowMicrobials/issues/new)
  (needs a free GitHub account), with the tool's GitHub link and what happened, or
- email **t.k.business@outlook.com**.

Both are also under **Settings › Report an issue** in the app.

**When a run fails**, its page has **Save a problem report**: one text file,
saved where you choose, with what is needed to find out why (the app and
computer, the workflow, what each step printed and where it stopped). It
holds no sequence data, and sample names and the computer's user name are
replaced before it is written. Attach it to your issue or email.

## For developers

The app is written in Rust (`rust/`): `gwm-core` (tools, workflows, the
compiler), `gwm-engine` (runs each tool in its container, resume, provenance),
`gwm-server` (the local API), `gwm-cli` (the `gwm` command), `rust/gui`
(Svelte) and `rust/desktop` (Tauri). Tools are in `tools/`, the built-in
workflow in `workflows/`, the database list in `databases/`.

```bash
cd rust
cargo test --workspace
cargo run -p gwm-cli -- serve --port 8642 --gui gui/dist   # after: cd gui && npm ci && npm run build
```

Installers are built by `.github/workflows/release.yml` when a `v2*` tag is pushed.

## Citing

If you use this app, please cite this repository (see
[`CITATION.cff`](CITATION.cff)) and the tools it runs (each tool's citation is
shown on the Tools page).

## License

MIT, see [`LICENSE`](LICENSE). The tools the app runs keep their own licences;
the app only pulls their published containers.
