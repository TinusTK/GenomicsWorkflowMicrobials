#!/bin/bash
# Starts the engine inside WSL for the Windows desktop app.
# $1 engine binary (in the installed app), $2 the app's resources folder,
# $3 the runs folder (Windows Documents), $4 port, $5 a name for this build.
# The engine runs from a copy on the Linux side, so an app update can replace
# the installed files while a run (using the old copy) is still going.
set -e
d="$HOME/.gwm/bin"
mkdir -p "$d"
if [ ! -x "$d/$5" ]; then
    cp "$1" "$d/$5.part"
    chmod +x "$d/$5.part"
    mv "$d/$5.part" "$d/$5"
fi
exec env GWM_ROOT="$2" GWM_RUNS_DIR="$3" "$d/$5" serve --port "$4"
