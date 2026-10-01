#!/bin/sh
# E7h review 2: does the memory cut reach #296's class? #296's input, as the
# issue generates it at 8, 16 and 32 KB, seeds markdown_inline in the
# `ubsan` arm at CI's smoke settings. The owner's ruling at E7h's fix round 1
# is that a parse past the memory cap fails the run; #296's 16 KB input alone
# under this build peaks at 5.29 GB in 33.6 s (8 KB: 2.21 GB, 4 KB: 1.13 GB),
# past the 4 GB cap and inside the 120 s limit. This exits 0 when the run
# fails on it and 1 when the run passes, which it did at 17c3c8f: each input
# crosses the first 1 GB limit, is minimized against that limit to 4,123
# bytes, and only the minimum is confirmed under the cap, where it returns.
# Usage, from a worktree: tests/e7h_review2/i296_tripwire.sh [OUTDIR]
set -u
out=${1:-${TMPDIR:-/tmp}/e7h-review2-i296}
mkdir -p "$out/seeds"
python3 - "$out/seeds" <<'PY'
import sys
l = '_' * 582 + 'a `_`_'
for n, name in ((14, "us-8k.md"), (28, "us-16k.md"), (56, "us-32k.md")):
    open(f"{sys.argv[1]}/{name}", "w").write("\n".join([l] * n) + "\n")
PY
scripts/fuzz-grammars --arm ubsan --grammar markdown_inline --from "issue296=$out/seeds" \
    --seconds 15 --jobs 1 --max-files 100 --max-seed-kb 16 --min-mutations 300 \
    --max-seconds 240 --out "$out/report" > "$out/run.log" 2>&1
st=$?
grep -h '^| markdown_inline\|^- \*\*markdown_inline' "$out/run.log"
if [ "$st" = 1 ] && grep -q 'exceeded memory cap' "$out/report/ubsan/report.md"; then
    echo "the run failed on #296's input with a memory cut, as the ruling asks"
    exit 0
fi
echo "the run exited $st on #296's input without a memory cut"
exit 1
