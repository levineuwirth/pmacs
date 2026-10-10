# ra_probe.py FILE MODE OUTDIR: drive /usr/bin/rust-analyzer directly on FILE.
# MODE: "open" (didOpen only), "open+hint", "open+semfull", "open+semrange",
# "open+all" (the three requests pmacs sends), "none" (initialize only).
# Records: rc, seconds from didOpen to exit, stderr tail, frames received.
import json, os, shutil, subprocess, sys, tempfile, threading, time

path, mode, outdir = sys.argv[1], sys.argv[2], sys.argv[3]
os.makedirs(outdir, exist_ok=True)
work = tempfile.mkdtemp(prefix="ra-", dir=os.environ.get("RA_PROBE_TMP"))
target = os.path.join(work, os.path.basename(path))
shutil.copyfile(path, target)
text = open(target).read()
uri = "file://" + target
err = open(os.path.join(outdir, "stderr.txt"), "wb")
t0 = time.monotonic()
child = subprocess.Popen(["/usr/bin/rust-analyzer"], cwd=work, stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=err, bufsize=0)
log = open(os.path.join(outdir, "wire.txt"), "w", buffering=1)
lock = threading.Lock()

def send(msg):
    body = json.dumps(msg).encode()
    try:
        child.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        child.stdin.flush()
    except (BrokenPipeError, OSError) as e:
        with lock:
            log.write(f"{time.monotonic()-t0:9.3f} send failed {e}\n")
        return
    with lock:
        log.write(f"{time.monotonic()-t0:9.3f} C->S {msg.get('method')} id={msg.get('id')}\n")

def reader():
    f = child.stdout
    while True:
        headers = b""
        while not headers.endswith(b"\r\n\r\n"):
            c = f.read(1)
            if not c:
                with lock:
                    log.write(f"{time.monotonic()-t0:9.3f} S->C EOF\n")
                return
            headers += c
        n = int([h for h in headers.split(b"\r\n") if h.lower().startswith(b"content-length")][0].split(b":")[1])
        body = f.read(n)
        m = json.loads(body)
        with lock:
            log.write(f"{time.monotonic()-t0:9.3f} S->C id={m.get('id')} method={m.get('method')} "
                      f"{'result' if 'result' in m else ''}{'error ' + m['error']['message'][:80] if 'error' in m else ''}\n")
        if "method" in m and "id" in m:  # server request: answer null
            send({"jsonrpc": "2.0", "id": m["id"], "result": None})

threading.Thread(target=reader, daemon=True).start()
send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
    "processId": os.getpid(), "rootUri": "file://" + work,
    "capabilities": {"general": {"positionEncodings": ["utf-8", "utf-16"]},
                     "window": {"workDoneProgress": True}}}})
time.sleep(0.2)
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
t_open = None
if mode != "none":
    send({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
        "textDocument": {"uri": uri, "languageId": "rust", "version": 1, "text": text}}})
    t_open = time.monotonic()
rng = {"start": {"line": 0, "character": 0}, "end": {"line": 1, "character": 0}}
reqs = {
    "hint": ("textDocument/inlayHint", {"textDocument": {"uri": uri}, "range": rng}),
    "semrange": ("textDocument/semanticTokens/range", {"textDocument": {"uri": uri}, "range": rng}),
    "semfull": ("textDocument/semanticTokens/full", {"textDocument": {"uri": uri}}),
}
names = {"open+hint": ["hint"], "open+semfull": ["semfull"], "open+semrange": ["semrange"],
         "open+all": ["hint", "semrange", "semfull"]}.get(mode, [])
for i, n in enumerate(names):
    meth, params = reqs[n]
    send({"jsonrpc": "2.0", "id": 10 + i, "method": meth, "params": params})
try:
    rc = child.wait(timeout=10)
except subprocess.TimeoutExpired:
    rc = None
t_end = time.monotonic()
if rc is None:
    # still alive after 10 s: end it politely, then measure that.
    send({"jsonrpc": "2.0", "id": 99, "method": "shutdown", "params": None})
    time.sleep(0.5)
    send({"jsonrpc": "2.0", "method": "exit", "params": None})
    try:
        rc2 = child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        child.kill(); rc2 = "killed"
    verdict = f"alive after 10.0 s; then shutdown+exit rc={rc2}"
else:
    verdict = f"exited rc={rc} at {t_end - (t_open or t0):.3f} s after didOpen ({t_end - t0:.3f} s after spawn)"
err.close()
tail = open(os.path.join(outdir, "stderr.txt"), "rb").read().decode(errors="replace").strip().splitlines()[-6:]
with open(os.path.join(outdir, "verdict.txt"), "w") as f:
    f.write(verdict + "\n")
print(f"{mode}: {verdict}")
for line in tail:
    print("   stderr:", line[:200])
shutil.rmtree(work, ignore_errors=True)
