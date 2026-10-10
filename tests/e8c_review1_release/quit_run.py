# quit_run.py: one classified quit run of a pmacs binary in a private tmux pane.
#
#   quit_run.py --bin DIR --file F --delay S --wait W --out DIR --label L
#               [--init LUA] [--path-prefix DIR] [--keys KEYS...]
#
# The pane runs `sh`, which runs the editor as an ordinary child: the editor
# does not lead the session, so its exit hangs nothing up (memory 45). Every
# ambient root, HOME included, is private and set inside the pane's command.
# Prints one JSON line: exit time after the keys, when the alternate screen
# went off, load at the keys, the editor's whole descendant tree before the
# keys and which of it is alive one second after the exit.
import argparse, json, os, shutil, subprocess, sys, time

ap = argparse.ArgumentParser()
ap.add_argument("--bin", required=True)
ap.add_argument("--file", required=True)
ap.add_argument("--delay", type=float, required=True)
ap.add_argument("--wait", type=float, default=20)
ap.add_argument("--out", required=True)
ap.add_argument("--label", required=True)
ap.add_argument("--init")
ap.add_argument("--path-prefix")
ap.add_argument("--keys", nargs="*", default=["C-x", "C-c"])
ap.add_argument("--screen-every", type=float, default=0)
ap.add_argument("--tee", action="store_true")
ap.add_argument("--mode", default="")
ap.add_argument("--diagnose-after", type=float, default=0)
ap.add_argument("--strace", action="store_true")
a = ap.parse_args()

out = os.path.abspath(os.path.join(a.out, a.label))
shutil.rmtree(out, ignore_errors=True)
root = os.path.join(out, "root")
dirs = {k: os.path.join(root, k) for k in ["home", "cfg", "data", "state", "cache", "pstate", "run", "tmp"]}
for d in dirs.values():
    os.makedirs(d)
os.chmod(dirs["run"], 0o700)
if a.init:
    os.makedirs(os.path.join(dirs["cfg"], "pmacs"))
    text = open(a.init).read().replace("@OUT@", out).replace("@FAKE@", os.path.join(a.bin, "pmacs_fake_lsp")).replace("@MODE@", a.mode)
    with open(os.path.join(dirs["cfg"], "pmacs", "init.lua"), "w") as f:
        f.write(text)
path = "/usr/local/bin:/usr/bin:" + os.path.expanduser("~/.cargo/bin")
if a.path_prefix:
    path = a.path_prefix + ":" + path
if a.tee:
    shim_dir = os.path.join(root, "bin")
    os.makedirs(shim_dir)
    shim = os.path.join(shim_dir, "rust-analyzer")
    with open(shim, "w") as f:
        f.write(f'#!/bin/sh\nexec python3 -I {os.path.dirname(os.path.abspath(__file__))}/ra_tee.py {out}/wire.txt "$@"\n')
    os.chmod(shim, 0o755)
    path = shim_dir + ":" + path
env = (f"HOME={dirs['home']} TERM=xterm-256color PATH={path} "
       f"CARGO_HOME={os.environ.get('CARGO_HOME', os.path.expanduser('~/.cargo'))} "
       f"RUSTUP_HOME={os.environ.get('RUSTUP_HOME', os.path.expanduser('~/.rustup'))} "
       f"XDG_CONFIG_HOME={dirs['cfg']} XDG_DATA_HOME={dirs['data']} XDG_STATE_HOME={dirs['state']} "
       f"XDG_CACHE_HOME={dirs['cache']} PMACS_STATE_HOME={dirs['pstate']} "
       f"XDG_RUNTIME_DIR={dirs['run']} TMPDIR={dirs['tmp']}")
sock = os.path.join(out, "tmux.sock")
wrap = (f"strace -f -tt -T -e trace=fsync,fdatasync,sync,syncfs,rename,renameat,renameat2 -o {out}/strace.txt "
        if a.strace else "")
cmd = (f"echo $$ > {out}/shell.pid; env -i {env} {wrap}{a.bin}/pmacs {a.file}; "
       f"echo rc=$? > {out}/exit.txt; sleep 120")

def tm(*args, check=False):
    return subprocess.run(["tmux", "-S", sock, "-f", "/dev/null", *args],
                          capture_output=True, text=True, check=check)

def table():
    rows = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,stat=,args="], capture_output=True, text=True).stdout
    t = {}
    for line in rows.splitlines():
        f = line.split(None, 3)
        if len(f) >= 3:
            t[int(f[0])] = (int(f[1]), f[2], f[3] if len(f) > 3 else "")
    return t

def descendants(pid):
    t = table()
    found = [pid]
    i = 0
    while i < len(found):
        for p, (pp, _, _) in t.items():
            if pp == found[i] and p not in found:
                found.append(p)
        i += 1
    return {p: t[p] for p in found[1:] if p in t}

def threads(pid):
    rows = []
    for tid in sorted(os.listdir(f"/proc/{pid}/task"), key=int):
        try:
            st = open(f"/proc/{pid}/task/{tid}/stat").read().split(") ", 1)[1].split()
            comm = open(f"/proc/{pid}/task/{tid}/comm").read().strip()
            wchan = open(f"/proc/{pid}/task/{tid}/wchan").read().strip()
            rows.append(f"{tid} {comm} state={st[0]} utime={st[11]} stime={st[12]} wchan={wchan}")
        except OSError:
            pass
    return rows

def diagnose(pid, outdir):
    with open(os.path.join(outdir, "threads.txt"), "w") as f:
        for i in range(3):
            f.write(f"--- sample {i} at +{time.monotonic() - t_keys:.2f}s\n")
            f.write("\n".join(threads(pid)) + "\n")
            time.sleep(1)
    data = os.path.join(outdir, "perf.data")
    subprocess.run(["perf", "record", "-q", "-F", "499", "--call-graph", "dwarf,32768", "-t", str(pid),
                    "-o", data, "--", "sleep", "3"], capture_output=True)
    rep = subprocess.run(["perf", "report", "-i", data, "--stdio", "--no-children", "--percent-limit", "2",
                          "-G"], capture_output=True, text=True)
    with open(os.path.join(outdir, "perf-report.txt"), "w") as f:
        f.write(rep.stdout + rep.stderr)

def alive(pid):
    try:
        with open(f"/proc/{pid}/stat") as f:
            return f.read().split(") ", 1)[1].split()[0] != "Z"
    except OSError:
        return False

tm("new-session", "-d", "-x", "120", "-y", "40", "sh", "-c", cmd, check=True)
t_spawn = time.monotonic()
while not os.path.exists(f"{out}/shell.pid"):
    time.sleep(0.005)
shell = int(open(f"{out}/shell.pid").read())
editor = None
while editor is None and time.monotonic() - t_spawn < 10:
    kids = [p for p, (pp, _, args) in table().items() if pp == shell and "pmacs" in args]
    editor = kids[0] if kids else None
    time.sleep(0.005)
assert editor, "the editor never started"
time.sleep(max(0, a.delay - (time.monotonic() - t_spawn)))
before = descendants(editor)
screen_before = tm("capture-pane", "-p").stdout
load = open("/proc/loadavg").read().split()[:3]
tm("send-keys", *a.keys)
t_keys = time.monotonic()
alt_off = None
screens = []
last_shot = 0
diagnosed = False
while alive(editor) and time.monotonic() - t_keys < a.wait:
    now = time.monotonic()
    if alt_off is None:
        r = tm("display", "-p", "#{alternate_on}").stdout.strip()
        if r == "0":
            alt_off = now - t_keys
    if a.diagnose_after and not diagnosed and now - t_keys >= a.diagnose_after:
        diagnosed = True
        screens.append((round(now - t_keys, 3), tm("capture-pane", "-p").stdout))
        diagnose(editor, out)
        continue
    if a.screen_every and now - last_shot >= a.screen_every:
        screens.append((round(now - t_keys, 3), tm("capture-pane", "-p").stdout))
        last_shot = now
    time.sleep(0.01)
exited = not alive(editor)
t_exit = time.monotonic() - t_keys
result = {"label": a.label, "delay": a.delay, "load": load, "editor": editor}
if exited:
    result["exit_ms"] = round(t_exit * 1000)
    result["alt_off_ms"] = None if alt_off is None else round(alt_off * 1000)
else:
    result["exit_ms"] = None
    result["no_exit_after_s"] = a.wait
    result["last_screen"] = [l for l in tm("capture-pane", "-p").stdout.splitlines() if l.strip()][-3:]
time.sleep(1)
after = table()
result["children_before"] = {str(p): args[:70] for p, (_, _, args) in before.items()}
result["alive_after_1s"] = {str(p): {"ppid": after[p][0], "args": after[p][2][:70]}
                            for p in before if p in after and alive(p)}
result["screen_after_exit"] = [l for l in tm("capture-pane", "-p").stdout.splitlines() if l.strip()]
if screens:
    result["screens"] = [(t, [l for l in s.splitlines() if l.strip()][-2:]) for t, s in screens]
with open(os.path.join(out, "screen-before.txt"), "w") as f:
    f.write(screen_before)
# Clean up: the editor if it is still running, then every descendant, then tmux.
for p in [editor, *before]:
    if alive(p):
        try:
            os.kill(p, 9)
        except OSError:
            pass
tm("kill-server")
print(json.dumps(result), flush=True)
