#!/usr/bin/env python3
"""E7h review 1: what the TUI's main thread spends on opening a file and on
each keystroke, at a real terminal size.

The main thread is the process's first task (/proc/PID/task/PID). "Idle"
means its CPU time has not moved for IDLE seconds. The probe reports:

  open_busy  -- wall seconds from spawn until the main thread first idles
  open_cpu   -- main-thread CPU seconds spent by then
  key_busy   -- per keystroke: wall seconds from sending 'x' until the main
                thread idles again (median and max over N keys)
  quit       -- seconds from C-x C-c (after undoing the keys) to exit

Every line carries the pty size and the file's byte count.
"""
import argparse, fcntl, os, select, signal, struct, tempfile, termios, time

TCK = os.sysconf("SC_CLK_TCK")


def set_size(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def main_cpu(pid):
    try:
        with open(f"/proc/{pid}/task/{pid}/stat") as f:
            parts = f.read().rsplit(")", 1)[1].split()
        return (int(parts[11]) + int(parts[12])) / TCK
    except OSError:
        return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("binary")
    ap.add_argument("file")
    ap.add_argument("--rows", type=int, default=40)
    ap.add_argument("--cols", type=int, default=120)
    ap.add_argument("--keys", type=int, default=5)
    ap.add_argument("--idle", type=float, default=0.6)
    ap.add_argument("--limit", type=float, default=300.0)
    a = ap.parse_args()

    size = os.path.getsize(a.file)
    state = tempfile.mkdtemp(prefix="kc-", dir=os.environ.get("PROBE_TMP", "/tmp"))
    for d in ("config", "data", "state", "cache", "run"):
        os.makedirs(os.path.join(state, d), mode=0o700)
    env = dict(os.environ)
    env.update(
        XDG_CONFIG_HOME=os.path.join(state, "config"), XDG_DATA_HOME=os.path.join(state, "data"),
        XDG_STATE_HOME=os.path.join(state, "state"), XDG_CACHE_HOME=os.path.join(state, "cache"),
        PMACS_STATE_HOME=os.path.join(state, "state"), XDG_RUNTIME_DIR=os.path.join(state, "run"),
        TERM="xterm-256color", HOME=state,
    )
    master, slave = os.openpty()
    set_size(slave, a.rows, a.cols)
    set_size(master, a.rows, a.cols)
    t0 = time.monotonic()
    pid = os.fork()
    if pid == 0:
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        for fd in (0, 1, 2):
            os.dup2(slave, fd)
        os.close(master)
        os.chdir(os.path.dirname(os.path.abspath(a.file)))
        os.execvpe(a.binary, [a.binary, os.path.basename(a.file)], env)
    os.close(slave)

    out = 0

    def pump(t):
        nonlocal out
        r, _, _ = select.select([master], [], [], t)
        if r:
            try:
                out += len(os.read(master, 65536))
            except OSError:
                pass

    def alive():
        w, _ = os.waitpid(pid, os.WNOHANG)
        return w == 0

    def wait_idle(start, deadline):
        """Wall time from `start` until the main thread's CPU rate over the
        last WINDOW s falls under RATE of one core; returns when that
        window began (the end of the busy stretch)."""
        WINDOW, RATE = 0.5, 0.12
        samples = []
        while time.monotonic() < deadline:
            pump(0.02)
            if not alive():
                return None, "died"
            c = main_cpu(pid)
            now = time.monotonic()
            if c is None:
                return None, "gone"
            samples.append((now, c))
            while samples and now - samples[0][0] > WINDOW:
                samples.pop(0)
            if now - start >= WINDOW and len(samples) > 5 and samples[-1][0] - samples[0][0] >= WINDOW * 0.9:
                if samples[-1][1] - samples[0][1] < RATE * WINDOW:
                    return max(0.0, samples[0][0] - start), "idle"
        return None, "limit"

    deadline = t0 + a.limit
    # wait for first output so "idle" is not the pre-exec instant
    while out == 0 and time.monotonic() < deadline and alive():
        pump(0.05)
    open_busy, why = wait_idle(t0, deadline)
    open_cpu = main_cpu(pid)
    lat = []
    kcpu = []
    status = why
    if why == "idle":
        for _ in range(a.keys):
            s = time.monotonic()
            os.write(master, b"x")
            c0 = main_cpu(pid)
            b, why = wait_idle(s, deadline)
            # a settle's walk may start after the parse returns on a worker:
            # watch 1.5 s more and extend through any later burst
            while b is not None:
                w0, cw = time.monotonic(), main_cpu(pid)
                late = False
                while time.monotonic() - w0 < 1.5:
                    pump(0.02)
                    if (main_cpu(pid) or cw) - cw > 0.1:
                        late = True
                        break
                if not late:
                    break
                b2, why = wait_idle(s, deadline)
                if b2 is None:
                    b = None
                    break
                b = b2
            if b is None:
                status = why
                break
            lat.append(b)
            kcpu.append((main_cpu(pid) or 0) - (c0 or 0))
    quit_s = None
    if status == "idle" and alive():
        os.write(master, b"\x1f" * len(lat))  # undo the typing
        wait_idle(time.monotonic(), deadline)
        q = time.monotonic()
        os.write(master, b"\x18\x03")
        while time.monotonic() < deadline:
            pump(0.05)
            w, st = os.waitpid(pid, os.WNOHANG)
            if w:
                quit_s = time.monotonic() - q
                status = f"exit {os.WEXITSTATUS(st)}" if os.WIFEXITED(st) else f"signal {os.WTERMSIG(st)}"
                break
    if alive():
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
        if quit_s is None:
            status = f"killed ({status})"
    s = sorted(lat)
    ks = (f"key_busy_median={s[len(s)//2]:.3f}s key_busy_max={s[-1]:.3f}s key_cpu_median={sorted(kcpu)[len(kcpu)//2]:.2f}s") if s else "key_busy=none"
    fmt = lambda v: "None" if v is None else f"{v:.2f}"
    print(f"{os.path.basename(a.file)} bytes={size} pty={a.rows}x{a.cols} open_busy={fmt(open_busy)}s "
          f"open_cpu={fmt(open_cpu)}s keys={len(lat)} {ks} quit={fmt(quit_s)}s {status}", flush=True)


if __name__ == "__main__":
    main()
