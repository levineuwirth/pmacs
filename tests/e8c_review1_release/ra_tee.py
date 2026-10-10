# ra_tee.py LOGFILE [args...]: relay stdio between the editor and
# /usr/bin/rust-analyzer, logging every frame's direction, time, id and
# method, and rust-analyzer's exit status. stderr passes through.
import json, os, subprocess, sys, threading, time

logpath = sys.argv[1]
t0 = time.monotonic()
log = open(logpath, "a", buffering=1)
lock = threading.Lock()
child = subprocess.Popen(["/usr/bin/rust-analyzer", *sys.argv[2:]], stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, bufsize=0)

def note(s):
    with lock:
        log.write(f"{time.time():.3f} +{time.monotonic()-t0:8.3f} {s}\n")

note(f"spawn ra pid={child.pid} tee pid={os.getpid()}")

def relay(src, dst, tag, close_dst):
    buf = b""
    while True:
        try:
            chunk = os.read(src, 65536)
        except OSError:
            chunk = b""
        if not chunk:
            note(f"{tag} EOF")
            if close_dst:
                try:
                    os.close(dst)
                except OSError:
                    pass
            return
        try:
            os.write(dst, chunk)
        except OSError as e:
            note(f"{tag} write error {e}")
        buf += chunk
        while True:
            h = buf.find(b"\r\n\r\n")
            if h < 0:
                break
            n = 0
            for line in buf[:h].split(b"\r\n"):
                if line.lower().startswith(b"content-length:"):
                    n = int(line.split(b":")[1])
            if len(buf) < h + 4 + n:
                break
            body, buf = buf[h + 4:h + 4 + n], buf[h + 4 + n:]
            try:
                m = json.loads(body)
                note(f"{tag} id={m.get('id')} method={m.get('method')}"
                     f"{' result' if 'result' in m else ''}{' error' if 'error' in m else ''}")
            except ValueError:
                note(f"{tag} unparsed")

threading.Thread(target=relay, args=(0, child.stdin.fileno(), "C->S", True), daemon=True).start()
threading.Thread(target=relay, args=(child.stdout.fileno(), 1, "S->C", False), daemon=True).start()
rc = child.wait()
note(f"ra exited rc={rc}")
time.sleep(0.2)
