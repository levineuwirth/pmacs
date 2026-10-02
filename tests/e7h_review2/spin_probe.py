#!/usr/bin/env python3
"""E7h review 2: does typing into a markdown buffer whose inline parse the
deadline cannot cut stack parses? Opens FILE in the TUI (40x120 pty, isolated
state, no language server), types N characters at the start of the buffer one
every 0.5 s, then after WAIT seconds samples every thread of the editor for
2 s and reports how many were on a CPU most of that window, with the
process's total CPU and RSS, then quits. Usage: spin_probe.py BINARY FILE [N] [WAIT]
"""
import fcntl, os, select, signal, struct, sys, tempfile, termios, time


def ticks(pid):
    out = {}
    for t in os.listdir(f"/proc/{pid}/task"):
        try:
            parts = open(f"/proc/{pid}/task/{t}/stat").read().rsplit(")", 1)[1].split()
            out[t] = int(parts[11]) + int(parts[12])
        except OSError:
            pass
    return out


binary, path = sys.argv[1], sys.argv[2]
n = int(sys.argv[3]) if len(sys.argv) > 3 else 5
wait = float(sys.argv[4]) if len(sys.argv) > 4 else 10.0
state = tempfile.mkdtemp(prefix="r2s-", dir=os.environ.get("PROBE_TMP", "/tmp"))
for d in ("config/pmacs", "data", "state", "cache", "run"):
    os.makedirs(os.path.join(state, d), mode=0o700)
open(f"{state}/config/pmacs/init.lua", "w").write("pmacs.lsp.config = {}\n")
env = dict(os.environ, XDG_CONFIG_HOME=f"{state}/config", XDG_DATA_HOME=f"{state}/data",
           XDG_STATE_HOME=f"{state}/state", XDG_CACHE_HOME=f"{state}/cache",
           PMACS_STATE_HOME=f"{state}/state", XDG_RUNTIME_DIR=f"{state}/run", TERM="xterm-256color")
m, s = os.openpty()
fcntl.ioctl(s, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
pid = os.fork()
if pid == 0:
    os.setsid(); fcntl.ioctl(s, termios.TIOCSCTTY, 0)
    for fd in (0, 1, 2):
        os.dup2(s, fd)
    os.execve(binary, [binary, path], env)
os.close(s)


def pump(secs):
    end = time.monotonic() + secs
    while time.monotonic() < end:
        r, _, _ = select.select([m], [], [], 0.05)
        if r:
            try:
                os.read(m, 65536)
            except OSError:
                return


pump(1.5)
for _ in range(n):
    os.write(m, b"z")
    pump(0.5)
pump(wait)
hz = os.sysconf("SC_CLK_TCK")
a = ticks(pid); pump(2.0); b = ticks(pid)
busy = sorted((b[t] - a.get(t, 0)) / hz / 2.0 for t in b)
spinning = sum(1 for x in busy if x > 0.5)
st = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
cpu = (int(st[11]) + int(st[12])) / hz
rss = next(int(l.split()[1]) for l in open(f"/proc/{pid}/status") if l.startswith("VmRSS:")) // 1024
print(f"{os.path.basename(path)} typed={n} after={wait}s threads={len(b)} spinning={spinning} "
      f"busiest={[round(x, 2) for x in busy[-4:]]} cpu_total={cpu:.1f}s rss={rss}MB", flush=True)
os.write(m, b"\x18\x03"); pump(1.0); os.write(m, b"n"); pump(0.5)
os.kill(pid, signal.SIGKILL); os.waitpid(pid, 0)
