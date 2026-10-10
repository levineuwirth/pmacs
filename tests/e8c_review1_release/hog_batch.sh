#!/bin/bash
# hog_batch.sh GROUP N_HOGS -- RUN_ARGS...: start N_HOGS CPU-bound busy loops
# (nice 0, pids recorded), run quit_run.py with RUN_ARGS for each label in
# $LABELS, then kill exactly the recorded hogs. Results to runs/GROUP/results.jsonl.
set -u
cd /home/jeans/build/e8c-review1
G=$1; N=$2; shift 2
[ "$1" = "--" ] && shift
OUT=runs/$G
mkdir -p "$OUT"
HOGS=()
for i in $(seq "$N"); do
  python3 -I -c 'while True: pass' &
  HOGS+=($!)
done
echo "${HOGS[@]}" > "$OUT/hogs.pids"
sleep 10
for L in $LABELS; do
  python3 -I scripts/quit_run.py --bin /home/jeans/build/e8c-review1/release --out "$OUT" --label "$L" "$@" \
    >> "$OUT/results.jsonl" 2>> "$OUT/errors.txt"
done
kill "${HOGS[@]}"
echo DONE >> "$OUT/done.txt"
