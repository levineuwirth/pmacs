#!/usr/bin/env python3
"""E7h review 1's pty probe. Opens the TUI on a file in a pseudo-terminal
of a real size (set on the slave before exec and on the master), isolated
state, and measures:

  first   -- seconds from spawn to the first output byte
  settle  -- seconds from spawn until output has been quiet for QUIET s
  quit    -- seconds from sending C-x C-c (at --quit-at s) to exit
  keys    -- optional: after settle, type N characters one at a time and
             report the median and max seconds until output answers each
  cpu     -- the process's user+sys CPU seconds at exit (from /proc)
  rss     -- its peak RSS in MB (VmHWM)

Every result line carries the pty size, the file's byte count and the exit
status, so an unsized or truncated run cannot pass for a measurement.
"""
import argparse, fcntl, os, select, signal, struct, sys, tempfile, termios, time


def set_size(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def get_size(fd):
    r, c, _, _ = struct.unpack("HHHH", fcntl.ioctl(fd, termios.TIOCGWINSZ, b"\0" * 8))
    return r, c


def proc_stat(pid):
    try:
        with open(f"/proc/{pid}/stat") as f:
            parts = f.read().rsplit(")", 1)[1].split()
        tck = os.sysconf("SC_CLK_TCK")
        cpu = (int(parts[11]) + int(parts[12])) / tck
    except OSError:
        cpu = None
    hwm = None
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmHWM:"):
                    hwm = int(line.split()[1]) / 1024
    except OSError:
        pass
    return cpu, hwm


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("binary")
    ap.add_argument("file")
    ap.add_argument("--rows", type=int, default=40)
    ap.add_argument("--cols", type=int, default=120)
    ap.add_argument("--quit-at", type=float, default=1.0)
    ap.add_argument("--quiet", type=float, default=1.5)
    ap.add_argument("--limit", type=float, default=240.0)
    ap.add_argument("--keys", type=int, default=0)
    ap.add_argument("--settle-first", action="store_true",
                    help="wait for settle before typing and quitting")
    a = ap.parse_args()

    size = os.path.getsize(a.file)
    state = tempfile.mkdtemp(prefix="pr1-", dir=os.environ.get("PROBE_TMP", "/tmp"))
    for d in ("config", "data", "state", "cache", "run"):
        os.makedirs(os.path.join(state, d), mode=0o700)
    env = dict(os.environ)
    env.update(
        XDG_CONFIG_HOME=os.path.join(state, "config"),
        XDG_DATA_HOME=os.path.join(state, "data"),
        XDG_STATE_HOME=os.path.join(state, "state"),
        XDG_CACHE_HOME=os.path.join(state, "cache"),
        PMACS_STATE_HOME=os.path.join(state, "state"),
        XDG_RUNTIME_DIR=os.path.join(state, "run"),
        TERM="xterm-256color",
        HOME=state,
    )
    master, slave = os.openpty()
    set_size(slave, a.rows, a.cols)
    set_size(master, a.rows, a.cols)
    rows, cols = get_size(slave)
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

    first = None
    last_out = None
    total = 0
    quit_sent = None
    settled = None
    status = None
    key_lat = []
    typing = a.keys > 0
    pending_key = None
    typed = 0
    peak_cpu, peak_rss = None, None

    def drain(timeout):
        nonlocal first, last_out, total
        r, _, _ = select.select([master], [], [], timeout)
        if r:
            try:
                data = os.read(master, 65536)
            except OSError:
                return None
            now = time.monotonic()
            if first is None:
                first = now - t0
            last_out = now
            total += len(data)
            if os.environ.get('PROBE_SAVE'):
                open(os.environ['PROBE_SAVE'], 'ab').write(data)
            return data
        return b""

    while True:
        now = time.monotonic()
        el = now - t0
        cpu, rss = proc_stat(pid)
        if cpu is not None:
            peak_cpu = cpu
        if rss is not None:
            peak_rss = rss
        wpid, st = os.waitpid(pid, os.WNOHANG)
        if wpid:
            status = st
            break
        if el > a.limit:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
            status = "killed-at-limit"
            break
        if settled is None and last_out is not None and now - last_out >= a.quiet:
            settled = last_out - t0
        ready_for_keys = settled is not None if a.settle_first else el >= a.quit_at
        if typing and ready_for_keys and quit_sent is None:
            if pending_key is None and typed < a.keys:
                os.write(master, b"x")
                pending_key = time.monotonic()
                typed += 1
            elif pending_key is not None and last_out is not None and last_out > pending_key:
                # answered; wait for the burst to finish before the next key
                if now - last_out > 0.15:
                    key_lat.append(last_out - pending_key)
                    pending_key = None
            elif pending_key is None and typed >= a.keys:
                typing = False
        if not typing and quit_sent is None and ready_for_keys:
            if a.keys > 0:
                # undo the typing so quit does not prompt: C-/ per key
                os.write(master, b"\x1f" * a.keys)
                time.sleep(0.3)
            os.write(master, b"\x18\x03")
            quit_sent = time.monotonic()
        if drain(0.05) is None:
            pass
    end = time.monotonic()
    quit_s = end - quit_sent if quit_sent else None
    if isinstance(status, int):
        if os.WIFEXITED(status):
            st = f"exit {os.WEXITSTATUS(status)}"
        else:
            st = f"signal {os.WTERMSIG(status)}"
    else:
        st = str(status)
    lat = ""
    if key_lat:
        s = sorted(key_lat)
        lat = f" keys={len(s)} key_median={s[len(s)//2]*1000:.0f}ms key_max={s[-1]*1000:.0f}ms"
    print(
        f"{os.path.basename(a.file)} bytes={size} pty={rows}x{cols} "
        f"first={first if first is None else round(first,2)} "
        f"settle={settled if settled is None else round(settled,2)} "
        f"quit_after={quit_s if quit_s is None else round(quit_s,2)} total={round(end-t0,2)} "
        f"cpu={peak_cpu} rss_mb={peak_rss if peak_rss is None else round(peak_rss)} out={total} {st}{lat}",
        flush=True,
    )


if __name__ == "__main__":
    main()
