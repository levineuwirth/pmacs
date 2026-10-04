#!/usr/bin/env python3
"""E7i review 1: what a user of a release archive without the parse worker
sees. Runs BINARY (a `pmacs` alone in its directory, as release.yml stages it,
or beside a worker for the control) on FILE in a 40x120 pty with isolated
state and no language server, for SECONDS; reports whether the screen carried
the editor's one-time notice, which foreground colours the file's text was
painted in, and the notice's text as the screen showed it.
Usage: archive_probe.py BINARY FILE [SECONDS]
"""
import fcntl, os, re, select, signal, struct, sys, tempfile, termios, time

binary, path = sys.argv[1], sys.argv[2]
seconds = float(sys.argv[3]) if len(sys.argv) > 3 else 4.0
state = tempfile.mkdtemp(prefix="arc-", dir=os.path.expanduser("~/build/e7i-review1/tmp"))
for d in ("config/pmacs", "data", "state", "cache", "run"):
    os.makedirs(os.path.join(state, d), exist_ok=True)
os.chmod(os.path.join(state, "run"), 0o700)
open(os.path.join(state, "config", "pmacs", "init.lua"), "w").write("pmacs.lsp.config = {}\n")
env = dict(os.environ, TERM="xterm-256color", HOME=state,
           XDG_CONFIG_HOME=os.path.join(state, "config"), XDG_DATA_HOME=os.path.join(state, "data"),
           XDG_STATE_HOME=os.path.join(state, "state"), XDG_CACHE_HOME=os.path.join(state, "cache"),
           PMACS_STATE_HOME=os.path.join(state, "state"), XDG_RUNTIME_DIR=os.path.join(state, "run"))
master, slave = os.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
pid = os.fork()
if pid == 0:
    os.setsid()
    fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    for fd in (0, 1, 2):
        os.dup2(slave, fd)
    os.execve(binary, [binary, path], env)
os.close(slave)
out = bytearray()
end = time.monotonic() + seconds
while time.monotonic() < end:
    r, _, _ = select.select([master], [], [], 0.05)
    if r:
        try:
            data = os.read(master, 65536)
        except OSError:
            break
        if not data:
            break
        out.extend(data)
os.kill(pid, signal.SIGKILL)
os.waitpid(pid, 0)
text = out.decode("utf-8", "replace")
colours = sorted(set(re.findall(r"\x1b\[[0-9;]*38;[25];[0-9;]+m", text)))
plain = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", text)
notice = re.search(r"syntax: [^\r\n]{0,200}", plain)
print(f"binary={binary} file={os.path.basename(path)} bytes_out={len(out)} "
      f"distinct_fg_sequences={len(colours)} notice={'yes' if notice else 'no'}")
if notice:
    print("notice: " + notice.group(0))
print("mentions pmacs-parse-unit on screen: " + str("pmacs-parse-unit" in plain))
for m in re.finditer(r"[^\r\n]{0,160}pmacs-parse-unit[^\r\n]{0,160}", plain):
    print("screen: " + m.group(0).strip())
    break
