#!/usr/bin/env python3
"""E7h review 2: one keystroke into a file, then quit, in a pseudo-terminal of
a real size (40x120, set on the slave before exec), with isolated state and
no language server (an init.lua emptying pmacs.lsp.config). Types `x` at 1 s,
reports whether the editor is alive 3 s later, sends C-x C-c and `y` to the
modified-buffer question, and reports the exit status and any abort text the
pty carried (glibc's malloc checks, a sanitizer, a signal). One line a run.
Usage: onekey.py BINARY FILE [RUNS]
"""
import fcntl, os, select, signal, struct, sys, tempfile, termios, time

ABORTS = (b"corrupted", b"malloc():", b"free():", b"double free", b"Aborted",
          b"AddressSanitizer", b"Segmentation", b"panicked", b"assertion")


def run(binary, path):
    state = tempfile.mkdtemp(prefix="r2k-", dir=os.environ.get("PROBE_TMP", "/tmp"))
    for d in ("config/pmacs", "data", "state", "cache", "run"):
        os.makedirs(os.path.join(state, d), exist_ok=True)
    os.chmod(os.path.join(state, "run"), 0o700)
    open(os.path.join(state, "config", "pmacs", "init.lua"), "w").write("pmacs.lsp.config = {}\n")
    env = dict(os.environ, TERM="xterm-256color",
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
    start = time.monotonic()

    def pump(until):
        while time.monotonic() < until:
            r, _, _ = select.select([master], [], [], 0.05)
            if r:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    return
                if not data:
                    return
                out.extend(data)

    pump(start + 1.0)
    os.write(master, b"x")
    pump(time.monotonic() + 3.0)
    done, status = os.waitpid(pid, os.WNOHANG)
    alive = done == 0
    if alive:
        os.write(master, b"\x18\x03")
        pump(time.monotonic() + 1.0)
        os.write(master, b"y")
        deadline = time.monotonic() + 10.0
        while time.monotonic() < deadline:
            pump(time.monotonic() + 0.1)
            done, status = os.waitpid(pid, os.WNOHANG)
            if done:
                break
        if not done:
            os.kill(pid, signal.SIGKILL)
            _, status = os.waitpid(pid, 0)
            exit_text = "killed at the 10 s limit"
        else:
            exit_text = None
    else:
        exit_text = None
    if exit_text is None:
        if os.WIFSIGNALED(status):
            exit_text = f"signal {os.WTERMSIG(status)}"
        else:
            exit_text = f"exit {os.WEXITSTATUS(status)}"
    abort = next((a.decode() for a in ABORTS if a in out), None)
    return f"{os.path.basename(path)} bytes={os.path.getsize(path)} pty=40x120 alive_after_key={alive} {exit_text} abort_text={abort} out={len(out)}"


if __name__ == "__main__":
    binary, path = sys.argv[1], sys.argv[2]
    for _ in range(int(sys.argv[3]) if len(sys.argv) > 3 else 1):
        print(run(binary, path), flush=True)
