#!/usr/bin/env python3
"""E7i review 1: each regress input and variant alone through a worker
(release by default), with a long deadline, timed: a hang is told from a
slow parse. Usage: time_regress.py DEADLINE_MS [UNIT]"""
import os, re, subprocess, sys, time
R1 = os.path.expanduser("~/build/e7i-review1")
H = f"{R1}/wt-target/debug/pmacs_grammar_fuzz"
U = sys.argv[2] if len(sys.argv) > 2 else f"{R1}/rel-target/release/pmacs-parse-unit"
DL = sys.argv[1]
R = "/home/jeans/Repos/personal/pmacs/fuzz/regress/markdown"
D = f"{R1}/regress-variants"
specs = [("markdown", f"{R}/296-underscores-16k.input"), ("markdown", f"{R}/296-asterisks-8k.input"),
         ("markdown", f"{R}/301-nested-openers-98.input"),
         ("markdown", f"{D}/orig-596.input"), ("markdown_inline", f"{D}/orig-596.input"),
         ("markdown", f"{D}/fence-596.input"), ("markdown", f"{D}/x-596.input"),
         ("markdown", f"{D}/orig-7672.input"), ("markdown_inline", f"{D}/orig-7672.input"),
         ("markdown", f"{D}/strip-7672.input"), ("markdown", f"{D}/fence-7672.input")]
for g, f in specs:
    out = f"{R1}/replay-rel/{os.path.basename(f)}-{g}-{DL}"
    subprocess.run(["rm", "-rf", out])
    t0 = time.monotonic()
    subprocess.run(["nice", "-n", "10", H, "replay-unit", "--unit", U, "--corpus", f"{R1}/empty", "--out", out,
                    "--deadline-ms", DL, "--jobs", "1", "--grammar", g, "--extra", f"{g}={f}"],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    took = time.monotonic() - t0
    rep = open(f"{out}/unit-report.md").read()
    m = re.search(r"contained: `[^`]*`, (.*)", rep)
    print(f"{g:16s} {os.path.basename(f):28s} {took:7.1f}s {m.group(1) if m else 'answered'}", flush=True)
