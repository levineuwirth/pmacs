#!/bin/bash
# load_batch.sh GROUP: the release-build quit runs, recorded to runs/GROUP/results.jsonl.
set -u
cd /home/jeans/build/e8c-review1
G=$1
OUT=runs/$G
mkdir -p "$OUT"
R=/home/jeans/build/e8c-review1/release
F=/home/jeans/build/e8c-review1/fixtures
run() { python3 -I scripts/quit_run.py --bin "$R" --out "$OUT" "$@" >> "$OUT/results.jsonl" 2>> "$OUT/errors.txt"; }
for i in 1 2 3 4 5 6; do run --file "$F/braces20000.rs" --delay 1 --wait 20 --label "ra1-$i"; done
for i in 1 2 3 4 5 6; do run --file "$F/braces20000.rs" --delay 5 --wait 20 --label "ra5-$i"; done
for i in 1 2; do run --file "$F/braces20000.rs" --delay 5 --wait 20 --label "tee5-$i" --tee; done
for i in 1 2 3; do run --file "$F/a.rs" --delay 3 --wait 20 --label "heir-$i" --init scripts/heir_init.lua --screen-every 0.5; done
for i in 1 2; do run --file "$F/a.rs" --delay 12 --wait 20 --label "crashheir-$i" --init scripts/heir_init.lua --mode abortonopen --screen-every 0.5; done
echo DONE >> "$OUT/done.txt"
