#!/usr/bin/env python3
"""E8b review 1, from E8 review 2's driver: the release TUI in a tmux pane.

tui_drive.py <name> <steps.json>

session.sh lays out the session and starts a release daemon; the release
`pmacs --attach` runs in a 120x40 tmux pane, its keys sent by
`tmux send-keys`, so they reach pmacs as terminal input (the production
key path). Captures land under ~/build/e8b-review1/witness/<name>/.

Steps (JSON list):
  ["keys", "C-x", "b", ...]   tmux send-keys, one key each, 0.15 s apart
  ["text", "abc"]              literal text
  ["sleep", 1.0]
  ["capture", "label"]         the pane, plain, with its last two lines
                               (mode line and band) in report.txt
"""
import json, os, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
R = "/home/jeans/build/cargo-target/release"
name, steps_file = sys.argv[1], sys.argv[2]
out = subprocess.run([f"{HERE}/session.sh", name], capture_output=True, text=True, check=True).stdout
vals = dict(kv.split("=", 1) for kv in out.split())
W, SOCK, DPID = vals["W"], vals["SOCK"], int(vals["DPID"])
env = {k: f"{W}/{v}" for k, v in [("XDG_CONFIG_HOME", "cfg"), ("XDG_DATA_HOME", "data"),
                                  ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache"),
                                  ("PMACS_STATE_HOME", "state"), ("TMPDIR", "tmp")]}
envs = " ".join(f"{k}={v}" for k, v in env.items())
S = f"e8br1-{name}"
subprocess.run(["tmux", "kill-session", "-t", S], stderr=subprocess.DEVNULL)
subprocess.run(["tmux", "new-session", "-d", "-s", S, "-x", "120", "-y", "40",
                f"env {envs} TERM=xterm-256color {R}/pmacs --attach --socket {SOCK}"], check=True)
time.sleep(2.5)
report = open(f"{W}/report.txt", "w")


def pane():
    return subprocess.run(["tmux", "capture-pane", "-p", "-t", S], capture_output=True, text=True).stdout


for step in json.load(open(steps_file)):
    kind = step[0]
    if kind == "keys":
        for k in step[1:]:
            subprocess.run(["tmux", "send-keys", "-t", S, k])
            time.sleep(0.15)
    elif kind == "text":
        subprocess.run(["tmux", "send-keys", "-t", S, "-l", step[1]])
        time.sleep(0.3)
    elif kind == "sleep":
        time.sleep(step[1])
    elif kind == "capture":
        time.sleep(0.6)
        text = pane()
        open(f"{W}/{step[1]}.txt", "w").write(text)
        tail = [l.rstrip() for l in text.rstrip("\n").split("\n")][-2:]
        report.write(f"{step[1]}: {tail!r}\n")
    report.flush()

subprocess.run(["tmux", "kill-session", "-t", S], stderr=subprocess.DEVNULL)
try:
    os.kill(DPID, 15)
    for _ in range(100):
        os.kill(DPID, 0)
        time.sleep(0.1)
    report.write("daemon still alive, KILL\n")
    os.kill(DPID, 9)
except ProcessLookupError:
    pass
report.write("DONE\n")
print(f"{W}/report.txt")
