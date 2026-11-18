#!/bin/bash
# Makes the copy of a Kraken2 database that Kun-peng reads: the one big hash
# table cut into pieces, so that only one piece at a time is held in memory.
#
#   prepare.sh <Kraken2 database folder> <folder for the Kun-peng copy>
#
# The copy stands on its own (the Kraken2 database can be removed afterwards).
# Safe to start twice at once (two samples of a run, a run and the Databases
# page): the second waits for the first and then finds the work done.
# Used by the Kun-peng step itself and by the app's "Prepare for Kun-peng".
set -eu

src=$1
dest=$2
# slots per piece: a piece is 4 bytes a slot, so 2 GB each
capacity=${KUN_PENG_HASH_CAPACITY:-512M}

if [ -f "$dest/hash_config.k2d" ]; then exit 0; fi

locks="$(dirname "$dest")/locks"
mkdir -p "$locks"
exec 9> "$locks/$(basename "$dest").lock"
flock 9
# made by another step while this one waited
if [ -f "$dest/hash_config.k2d" ]; then exit 0; fi

for f in hash.k2d opts.k2d taxo.k2d; do
    if [ ! -f "$src/$f" ]; then
        echo "ERROR: $src is not a Kraken2 database ($f is missing)" >&2
        exit 1
    fi
done

need_kb=$(( $(stat -L -c %s "$src/hash.k2d") / 1024 + $(stat -L -c %s "$src/taxo.k2d") / 1024 + 1024 ))
free_kb=$(df -Pk "$(dirname "$dest")" | awk 'NR == 2 {print $4}')
if [ "$free_kb" -lt "$need_kb" ]; then
    echo "ERROR: not enough free disk space for the Kun-peng copy: $(( need_kb / 1048576 + 1 )) GB needed, $(( free_kb / 1048576 )) GB free" >&2
    exit 1
fi

part="$dest.partial"
rm -rf "$part"
mkdir -p "$part"
# kun_peng writes the pieces beside the hash.k2d it reads: give it a link, not a second copy
ln -s "$src/hash.k2d" "$part/hash.k2d"
cp "$src/opts.k2d" "$src/taxo.k2d" "$part/"
# Bracken's small k-mer files come along: its estimate then works from the copy alone
for f in "$src"/database*mers.kmer_distrib; do
    if [ -f "$f" ]; then cp "$f" "$part/"; fi
done
echo "Preparing $(basename "$src") for Kun-peng (done once)" >&2
kun_peng hashshard --db "$part" --hash-capacity "$capacity" >&2
rm -f "$part/hash.k2d"
if [ ! -f "$part/hash_config.k2d" ] || [ ! -f "$part/hash_1.k2d" ]; then
    echo "ERROR: kun_peng did not write the pieces of the database" >&2
    rm -rf "$part"
    exit 1
fi
# which Kraken2 database this is a copy of (its folder name carries the date)
basename "$src" > "$part/source.txt"
rm -rf "$dest"
mv "$part" "$dest"
echo "The Kun-peng copy is ready" >&2
