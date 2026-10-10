# daemon_run.py --bin DIR --out DIR --label L --init LUA [--mode M] --delay S --wait W
#
# A release daemon with private roots opens a file from its init.lua; a TUI
# frontend attaches in a private tmux pane and, after DELAY seconds, sends
# C-x C-c. Records when the frontend exits, when the daemon exits, and
# which of the daemon's descendants are alive one second after.
import argparse, json, os, shutil, subprocess, time

ap = argparse.ArgumentParser()
ap.add_argument("--bin", required=True)
ap.add_argument("--out", required=True)
ap.add_argument("--label", required=True)
ap.add_argument("--init", required=True)
ap.add_argument("--mode", default="")
ap.add_argument("--file", required=True)
ap.add_argument("--delay", type=float, default=5)
ap.add_argument("--wait", type=float, default=30)
a = ap.parse_args()

out = os.path.abspath(os.path.join(a.out, a.label))
shutil.rmtree(out, ignore_errors=True)
root = os.path.join(out, "root")
dirs = {k: os.path.join(root, k) for k in ["home", "cfg", "data", "state", "cache", "pstate", "run", "tmp"]}
for d in dirs.values():
    os.makedirs(d)
os.chmod(dirs["run"], 0o700)
os.makedirs(os.path.join(dirs["cfg"], "pmacs"))
text = (open(a.init).read().replace("@OUT@", out).replace("@FAKE@", os.path.join(a.bin, "pmacs_fake_lsp"))
        .replace("@MODE@", a.mode)) + f"\npmacs.buffer.find_or_open({a.file!r})\n"
open(os.path.join(dirs["cfg"], "pmacs", "init.lua"), "w").write(text)
sockdir = os.path.join(dirs["run"], "s")
os.makedirs(sockdir)
os.chmod(sockdir, 0o700)
sockpath = os.path.join(sockdir, "d.sock")
env = {"HOME": dirs["home"], "TERM": "xterm-256color",
       "PATH": "/usr/local/bin:/usr/bin:" + os.path.expanduser("~/.cargo/bin"),
       "CARGO_HOME": os.path.expanduser("~/.cargo"), "RUSTUP_HOME": os.path.expanduser("~/.rustup"),
       "XDG_CONFIG_HOME": dirs["cfg"], "XDG_DATA_HOME": dirs["data"], "XDG_STATE_HOME": dirs["state"],
       "XDG_CACHE_HOME": dirs["cache"], "PMACS_STATE_HOME": dirs["pstate"], "XDG_RUNTIME_DIR": dirs["run"],
       "TMPDIR": dirs["tmp"]}
derr = open(os.path.join(out, "daemon.stderr"), "w")
daemon = subprocess.Popen([os.path.join(a.bin, "pmacs"), "--daemon", "--socket", sockpath], env=env,
                          stdin=subprocess.DEVNULL, stdout=derr, stderr=derr, start_new_session=True)
t0 = time.monotonic()
while not os.path.exists(sockpath):
    assert daemon.poll() is None, "daemon died: " + open(os.path.join(out, "daemon.stderr")).read()
    assert time.monotonic() - t0 < 20, "no socket"
    time.sleep(0.02)

sock = os.path.join(out, "tmux.sock")
envs = " ".join(f"{k}={v}" for k, v in env.items())
cmd = f"echo $$ > {out}/shell.pid; env -i {envs} {a.bin}/pmacs --attach --socket {sockpath}; echo rc=$? > {out}/attach-exit.txt; sleep 120"
def tm(*args):
    return subprocess.run(["tmux", "-S", sock, "-f", "/dev/null", *args], capture_output=True, text=True)
tm("new-session", "-d", "-x", "120", "-y", "40", "sh", "-c", cmd)

def table():
    rows = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,args="], capture_output=True, text=True).stdout
    t = {}
    for line in rows.splitlines():
        f = line.split(None, 2)
        if len(f) >= 2:
            t[int(f[0])] = (int(f[1]), f[2] if len(f) > 2 else "")
    return t

def descendants(pid):
    t = table(); found = [pid]; i = 0
    while i < len(found):
        for p, (pp, _) in t.items():
            if pp == found[i] and p not in found:
                found.append(p)
        i += 1
    return {p: t[p][1] for p in found[1:] if p in t}

def alive(pid):
    try:
        return open(f"/proc/{pid}/stat").read().split(") ", 1)[1].split()[0] != "Z"
    except OSError:
        return False

while not os.path.exists(f"{out}/shell.pid"):
    time.sleep(0.01)
shell = int(open(f"{out}/shell.pid").read())
time.sleep(a.delay)
front = [p for p, (pp, args) in table().items() if pp == shell and "pmacs" in args]
before = descendants(daemon.pid)
load = open("/proc/loadavg").read().split()[:3]
screen_before = tm("capture-pane", "-p").stdout
tm("send-keys", "C-x", "C-c")
t_keys = time.monotonic()
front_exit = daemon_exit = None
while time.monotonic() - t_keys < a.wait and (daemon_exit is None or front_exit is None):
    now = time.monotonic() - t_keys
    if front_exit is None and front and not alive(front[0]):
        front_exit = now
    if daemon_exit is None and daemon.poll() is not None:
        daemon_exit = now
    time.sleep(0.01)
time.sleep(1)
after = table()
res = {"label": a.label, "load": load, "front_exit_ms": None if front_exit is None else round(front_exit * 1000),
       "daemon_exit_ms": None if daemon_exit is None else round(daemon_exit * 1000),
       "daemon_rc": daemon.poll(),
       "daemon_children_before": list(before.values()),
       "alive_after_1s": {str(p): [after[p][0], after[p][1][:60]] for p in before if p in after and alive(p)},
       "screen_last_lines_before": [l for l in screen_before.splitlines() if l.strip()][-2:],
       "screen_after": [l for l in tm("capture-pane", "-p").stdout.splitlines() if l.strip()][-3:]}
for p in [daemon.pid, *before, *front]:
    if alive(p):
        try:
            os.kill(p, 9)
        except OSError:
            pass
tm("kill-server")
print(json.dumps(res), flush=True)
