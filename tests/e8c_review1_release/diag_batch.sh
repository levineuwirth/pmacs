#!/bin/bash
# diag_batch.sh GROUP: wait for gate 3's sweep, then six quit-at-1 s runs on
# the deep file, diagnosed at 4 s past the keys, waiting up to 90 s each.
set -u
cd /home/jeans/build/e8c-review1
until grep -q '^gate: \[04\] build *ok' /home/jeans/.claude/jobs/3d5783f5/tmp/gate-load3.out; do sleep 1; done
sleep 15
OUT=runs/$1; mkdir -p "$OUT"
for i in 1 2 3 4 5 6; do
  python3 -I scripts/quit_run.py --bin /home/jeans/build/e8c-review1/release --file /home/jeans/build/e8c-review1/fixtures/braces20000.rs \
    --delay 1 --wait 90 --diagnose-after 4 --out "$OUT" --label "q$i" >> "$OUT/results.jsonl" 2>> "$OUT/errors.txt"
done
echo DONE > "$OUT/done.txt"
