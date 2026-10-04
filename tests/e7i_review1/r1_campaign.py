#!/usr/bin/env python3
"""E7i review 1's re-measurement: E7i.1's campaign shape (40 samples a run,
six interleaved rounds a file, arm order shuffled per round), with the built
tip (`fa176de`, release) and the comparison's prototype (`9d0585f`, its
retained binaries) as arms beside today's native parse in each build. Calls
r1_run.py once per run; every run appends a JSON line to OUT/runs.jsonl and
keeps its directory under OUT."""
import os, random, subprocess, sys, time
OUT = sys.argv[1]
SEED = int(sys.argv[2]) if len(sys.argv) > 2 else 20261003
PARTS = sys.argv[3].split(",") if len(sys.argv) > 3 else ["latency", "patho", "steady", "accept"]
rng = random.Random(SEED)
RUN = [sys.executable, os.path.expanduser("~/build/e7i-review1/r1_run.py")]
os.makedirs(OUT, exist_ok=True)
LOG = open(os.path.join(OUT, "campaign.log"), "a")
LAT_ARMS = [("built", "none"), ("built", "process"), ("proto", "process"), ("proto", "none")]

def run(args):
    t0 = time.time()
    p = subprocess.run(["nice", "-n", "10"] + RUN + args + ["--out", OUT], capture_output=True, text=True)
    line = (f"{time.strftime('%H:%M:%S')} load={os.getloadavg()[0]:.1f} {time.time()-t0:6.1f}s "
            f"{' '.join(args)} -> {p.stdout.strip()[-400:]} {p.stderr.strip()[-300:] if p.returncode else ''}")
    print(line, file=LOG, flush=True)

print(f"seed {SEED} parts {PARTS}", file=LOG, flush=True)
if "latency" in PARTS:
    for r in range(6):
        for f in rng.sample(["rust", "markdown"], 2):
            for impl, arm in rng.sample(LAT_ARMS, len(LAT_ARMS)):
                run(["latency", "--impl", impl, "--arm", arm, "--file", f, "--samples", "40"])
if "patho" in PARTS:
    for r in range(3):
        for v in rng.sample(["296-16k", "301"], 2):
            arms = [("built", "process"), ("proto", "process")]
            if v == "301":
                arms.append(("built", "none"))
            for impl, arm in rng.sample(arms, len(arms)):
                run(["patho", "--impl", impl, "--arm", arm, "--victim", v, "--touch", "--samples", "30"])
if "steady" in PARTS:
    for r in range(3):
        for impl, arm in rng.sample([("built", "none"), ("built", "process"), ("proto", "process")], 3):
            run(["steady", "--impl", impl, "--arm", arm, "--samples", "20"])
if "accept" in PARTS:
    for victim in ["296-16k", "301", "nest-3x8k", "296-32k"]:
        run(["patho", "--impl", "built", "--arm", "process", "--victim", victim, "--once",
             "--samples", "5", "--tail-s", "6"])
    run(["patho", "--impl", "built", "--arm", "process", "--victim", ",".join(["296-16k"] * 6),
         "--once", "--total-mb", "2048", "--samples", "5", "--tail-s", "6"])
print("done", file=LOG, flush=True)
