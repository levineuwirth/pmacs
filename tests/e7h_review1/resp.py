#!/usr/bin/env python3
"""E7h review 1: how long the TUI takes to answer the user, at a real size.

Opens FILE at 40x120 with isolated state (and, with --no-lsp, an init.lua
setting pmacs.lsp.config = {} so no language server starts). At 1 s it
sends M-x and times how long the prompt takes to appear ("open"), then
C-g. Then, N times: types one character, waits WAIT seconds, sends M-x
and times the prompt ("key"), then C-g. A prompt that does not appear
within LIMIT seconds is reported as >LIMIT and the run stops there.
"""
import argparse, fcntl, os, select, signal, struct, tempfile, termios, time

ap = argparse.ArgumentParser()
ap.add_argument("binary"); ap.add_argument("file")
ap.add_argument("--keys", type=int, default=3)
ap.add_argument("--wait", type=float, default=0.3)
ap.add_argument("--limit", type=float, default=120.0)
ap.add_argument("--no-lsp", action="store_true")
a = ap.parse_args()

state = tempfile.mkdtemp(prefix="rs-", dir=os.environ.get("PROBE_TMP", "/tmp"))
for d in ("config/pmacs", "data", "state", "cache", "run"):
    os.makedirs(os.path.join(state, d), mode=0o700)
if a.no_lsp:
    open(f"{state}/config/pmacs/init.lua", "w").write("pmacs.lsp.config = {}\n")
env = dict(os.environ, XDG_CONFIG_HOME=f"{state}/config", XDG_DATA_HOME=f"{state}/data",
           XDG_STATE_HOME=f"{state}/state", XDG_CACHE_HOME=f"{state}/cache",
           PMACS_STATE_HOME=f"{state}/state", XDG_RUNTIME_DIR=f"{state}/run",
           TERM="xterm-256color", HOME=state)
m, s = os.openpty()
for fd in (s, m):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
rows, cols = struct.unpack("HHHH", fcntl.ioctl(s, termios.TIOCGWINSZ, b"\0" * 8))[:2]
t0 = time.monotonic()
pid = os.fork()
if pid == 0:
    os.setsid(); fcntl.ioctl(s, termios.TIOCSCTTY, 0)
    for fd in (0, 1, 2):
        os.dup2(s, fd)
    os.close(m); os.chdir(os.path.dirname(os.path.abspath(a.file)))
    os.execvpe(a.binary, [a.binary, os.path.basename(a.file)], env)
os.close(s)
buf = bytearray()
dead = False

def pump(t):
    global dead
    end = time.monotonic() + t
    while time.monotonic() < end and not dead:
        r, _, _ = select.select([m], [], [], 0.01)
        if r:
            try:
                buf.extend(os.read(m, 65536))
            except OSError:
                dead = True

def ask():
    """Send M-x; seconds until 'M-x' appears in new output, or None."""
    start = len(buf)
    os.write(m, b"\x1bx")
    t = time.monotonic()
    while time.monotonic() - t < a.limit and not dead:
        pump(0.01)
        if buf.find(b"M-x", start) >= 0:
            lat = time.monotonic() - t
            os.write(m, b"\x07")
            pump(0.2)
            return lat
    return None

pump(max(0.0, 1.0 - (time.monotonic() - t0)))
fmt = lambda v: f">{a.limit:.0f}" if v is None else f"{v:.3f}"
res = {"open": ask()}
keys = []
if res["open"] is not None:
    for _ in range(a.keys):
        os.write(m, b"x")
        pump(a.wait)
        lat = ask()
        keys.append(lat)
        if lat is None:
            break
os.kill(pid, signal.SIGKILL)
os.waitpid(pid, 0)
ks = " ".join(fmt(k) for k in keys)
print(f"{os.path.basename(a.file)} bytes={os.path.getsize(a.file)} pty={rows}x{cols} "
      f"lsp={'off' if a.no_lsp else 'on'} open_mx={fmt(res['open'])}s key_mx=[{ks}]s "
      f"{'died' if dead else 'alive'}", flush=True)
