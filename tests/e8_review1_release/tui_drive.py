#!/usr/bin/env python3
"""E8 review 1: drive the release TUI (built at 1ace8b3) in a tmux pane.

tui_drive.py <name> <file> <steps.json>

A release daemon visits <file> with real servers and isolated XDG roots;
the release `pmacs --attach` runs in a 120x40 tmux pane; each step runs
in order and captures land under ~/build/e8-review1/witness/<name>/.

Steps (JSON list):
  ["keys", "C-f", "C-f", ...]     tmux send-keys, one key each, 0.15 s apart
  ["text", "abc"]                  literal text
  ["raw", "\\u001b[<65;30;5M"]     raw bytes as terminal input (SGR mouse)
  ["ask", "h", "needle", 240]      C-c <k> every 2 s until the pane shows needle
  ["wait", "needle", 30]           wait until the pane shows needle
  ["sleep", 1.0]
  ["capture", "label"]             plain and SGR (-e) captures
"""
import json, os, subprocess, sys, time

R = "/home/jeans/build/e8-review1/rel-target/release"
name, path, steps_file = sys.argv[1], sys.argv[2], sys.argv[3]
W = f"/home/jeans/build/e8-review1/witness/{name}"
if os.path.exists(W):
    sys.exit(f"exists: {W}")
for d in ["run", "cfg/pmacs", "data", "state", "cache", "tmp"]:
    os.makedirs(f"{W}/{d}")
os.chmod(f"{W}/run", 0o700)
extra = os.environ.get("EXTRA", "")
with open(f"{W}/cfg/pmacs/init.lua", "w") as f:
    f.write(
        f"pmacs.buffer.find_or_open([[{path}]])\n"
        "for _, b in ipairs(pmacs.buffer.list()) do\n"
        "  if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\n"
        f"end\n{extra}\n"
    )
env = dict(os.environ)
env.update(
    XDG_CONFIG_HOME=f"{W}/cfg", XDG_DATA_HOME=f"{W}/data", XDG_STATE_HOME=f"{W}/state",
    XDG_CACHE_HOME=f"{W}/cache", PMACS_STATE_HOME=f"{W}/state", TMPDIR=f"{W}/tmp",
    CARGO_TARGET_DIR="/home/jeans/build/e8-review1/ra-target",
)
log = open(f"{W}/daemon.log", "w")
daemon = subprocess.Popen(["nice", "-n", "10", f"{R}/pmacs", "--daemon", "--socket", f"{W}/run/w.sock"],
                          env=env, stdout=log, stderr=log)
for _ in range(100):
    if os.path.exists(f"{W}/run/w.sock"):
        break
    time.sleep(0.1)
S = f"e8r1-{name}"
envs = " ".join(f"{k}={env[k]}" for k in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME",
                                         "XDG_CACHE_HOME", "PMACS_STATE_HOME", "TMPDIR", "CARGO_TARGET_DIR"])
subprocess.run(["tmux", "kill-session", "-t", S], stderr=subprocess.DEVNULL)
subprocess.run(["tmux", "new-session", "-d", "-s", S, "-x", "120", "-y", "40",
                f"env {envs} TERM=xterm-256color {R}/pmacs --attach --socket {W}/run/w.sock"], check=True)
time.sleep(2)
report = open(f"{W}/report.txt", "w")


def pane(esc=False):
    args = ["tmux", "capture-pane", "-p", "-t", S] + (["-e"] if esc else [])
    return subprocess.run(args, capture_output=True, text=True).stdout


def send(*keys):
    subprocess.run(["tmux", "send-keys", "-t", S, *keys])


for step in json.load(open(steps_file)):
    kind = step[0]
    if kind == "keys":
        for k in step[1:]:
            send(k)
            time.sleep(0.15)
    elif kind == "text":
        send("-l", step[1])
    elif kind == "raw":
        send("-l", step[1])
    elif kind == "sleep":
        time.sleep(step[1])
    elif kind in ("ask", "wait"):
        if kind == "ask":
            key, needle, cap = step[1], step[2], step[3]
        else:
            key, needle, cap = None, step[1], step[2]
        start = time.time(); found = False
        while time.time() - start < cap:
            if key:
                send("C-c"); time.sleep(0.1); send(key)
            time.sleep(2 if key else 0.5)
            if needle in pane():
                found = True
                break
        report.write(f"{kind} {key} {needle!r}: found={found} after={time.time() - start:.1f}s\n")
    elif kind == "capture":
        label = step[1]
        open(f"{W}/{label}.txt", "w").write(pane())
        open(f"{W}/{label}-esc.txt", "w").write(pane(esc=True))
        report.write(f"captured {label}\n")
    report.flush()

subprocess.run(["tmux", "kill-session", "-t", S], stderr=subprocess.DEVNULL)
daemon.terminate()
try:
    daemon.wait(10)
except subprocess.TimeoutExpired:
    report.write("daemon still alive, KILL\n")
    daemon.kill()
report.write("DONE\n")
