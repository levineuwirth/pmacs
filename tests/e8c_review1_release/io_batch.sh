#!/bin/bash
# io_batch.sh GROUP: a bounded disk writer (2 GB files written and fdatasync'd
# in a loop, in this scratch tree, pid recorded), then strace'd quits.
set -u
cd /home/jeans/build/e8c-review1
OUT=runs/$1; mkdir -p "$OUT" io
( for i in $(seq 1 12); do dd if=/dev/zero of=io/big.$((i % 2)) bs=1M count=2048 conv=fdatasync status=none; done ) &
W=$!
echo $W > "$OUT/writer.pid"
sleep 5
for L in a1 a2 a3 a4; do
  python3 -I scripts/quit_run.py --bin /home/jeans/build/e8c-review1/release --file /home/jeans/build/e8c-review1/fixtures/braces20000.rs \
    --delay 3 --wait 90 --strace --out "$OUT" --label "$L" >> "$OUT/results.jsonl" 2>> "$OUT/errors.txt"
done
kill $W 2>/dev/null
pkill -P $W 2>/dev/null
wait $W 2>/dev/null
echo DONE > "$OUT/done.txt"
