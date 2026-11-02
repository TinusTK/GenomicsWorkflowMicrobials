#!/usr/bin/env bash
# NTG pathology genomics pipeline - installer for Linux and Ubuntu on WSL2.
#   curl -fsSL https://raw.githubusercontent.com/TinusTK/NTGPathologyGenomicsWorkflowPipeline/main/install.sh | bash
#   ... | bash -s -- <folder>          install into <folder> (default ~/NTGPathologyGenomicsWorkflowPipeline)
#
# Installs the tools (git, Python + PyYAML, Java 21, Docker Engine, Nextflow) with
# apt, then clones the newest release of the pipeline. Running it again is safe:
# what is already there is kept. NTG_SKIP_PACKAGES=1 skips the tools (when you
# installed them yourself, or have no apt).
set -euo pipefail

REPO="https://github.com/TinusTK/NTGPathologyGenomicsWorkflowPipeline.git"
DEST="${1:-$HOME/NTGPathologyGenomicsWorkflowPipeline}"
say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

# All in a function, so bash has read the whole script before anything runs
# (with curl | bash, a command reading stdin could otherwise eat the rest of it).
main() {
if [ "$(id -u)" -eq 0 ]; then
    echo "Run this as your normal user, not root (it asks for sudo when needed)." >&2
    exit 1
fi

if [ "${NTG_SKIP_PACKAGES:-0}" != "1" ]; then
    if ! command -v apt-get >/dev/null; then
        echo "No apt-get here. Install git, python3 + PyYAML, Java 17+, Docker (or Apptainer) and" >&2
        echo "Nextflow yourself, then run this again with NTG_SKIP_PACKAGES=1." >&2
        exit 1
    fi
    say "Installing the tools (asks for your password)"
    sudo apt-get update
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y git python3 python3-yaml openjdk-21-jre-headless docker.io curl
    if ! id -nG "$USER" | grep -qw docker; then
        sudo usermod -aG docker "$USER"
        NEW_DOCKER_GROUP=1
    fi
    if command -v systemctl >/dev/null && [ -d /run/systemd/system ]; then
        sudo systemctl enable --now docker
    else
        echo "systemd isn't running, so Docker can't start by itself. On WSL, turn on systemd" >&2
        echo "(https://learn.microsoft.com/windows/wsl/systemd), or start Docker by hand." >&2
    fi
    if ! command -v nextflow >/dev/null; then
        say "Installing Nextflow"
        tmp=$(mktemp -d)
        (cd "$tmp" && curl -fsSL https://get.nextflow.io | bash)
        sudo install -m 755 "$tmp/nextflow" /usr/local/bin/nextflow
        rm -rf "$tmp"
    fi
fi

if [ -d "$DEST/.git" ]; then
    say "The pipeline is already in $DEST (kept as it is; update it from This computer -> Updates)"
else
    say "Getting the pipeline into $DEST"
    git clone --quiet "$REPO" "$DEST"
    # Sit on the newest release; the branch still follows GitHub, so in-app updates work.
    release=$(git -C "$DEST" tag --sort=-v:refname --list 'v*' | grep -E '^v[0-9]+(\.[0-9]+)*$' | head -n 1 || true)
    if [ -n "$release" ]; then
        git -C "$DEST" reset --quiet --hard "$release"
        echo "Release $release"
    fi
fi

say "Checking this computer"
"$DEST/ntg" hardware || true

say "Done"
echo "Start the UI:   $DEST/ntg ui     (then open http://localhost:8642)"
echo "In the UI, go to Databases and install CheckM and Kraken2 Standard-8."
if [ "${NEW_DOCKER_GROUP:-0}" = "1" ]; then
    echo
    echo "You were added to the docker group: log out and back in (on WSL: close the Ubuntu"
    echo "window and open it again) before the first run."
fi
}

main "$@"
