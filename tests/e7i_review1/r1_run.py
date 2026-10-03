#!/usr/bin/env python3
"""E7i review 1: E7i.1's measurement driver, with an --impl arm (the built tip or the prototype).
E7i.1's measurement driver: one release daemon per run, one arm each.

A run starts `pmacs --daemon` with its own five storage roots, an init.lua
that sets `syntax.isolation` (and the memory limits) and opens the input,
`PMACS_E7I_TRACE` pointing at a trace file, then drives the headless
`pmacs-gpu` latency probe against it and keeps the report, the trace, the
daemon's stderr and the memory figures. Nothing here parses or measures a
parse itself: every number comes from the daemon's real parse path, read
through the probe (keystroke to visible update) or /proc (memory).

Usage: e7i_run.py latency --arm none|wasm|process --file rust|markdown|latex
                          --samples N --out DIR
"""
import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import time

E7I = os.path.expanduser("~/build/e7i")
R1 = os.path.expanduser("~/build/e7i-review1")
PROBE = os.path.join(E7I, "measured-binaries", "pmacs-gpu")
IMPLS = {"built": os.path.join(R1, "rel-target/release"), "proto": os.path.join(E7I, "measured-binaries")}
BIN = None
INPUTS = os.path.join(E7I, "inputs")

# What to type where, per input: a keystroke that restyles the byte it is
# typed at, so the probe's `visible` marks the frame styled from the
# keystroke's own parse.
FILES = {
    # `//` before `let previous = ...` on line 63: the line becomes a comment.
    "rust": ("editor.rs", "//", "\n        let previous = self.0.replace(Some(frontend_id));", 9),
    # `# ` before the note's first paragraph: it becomes a heading.
    "markdown": ("roadmap.md", "# ", "\nPlanning note for [[pmacs]]", 1),
    # A short markdown file, for the instrument's own checks.
    # `%` before `\usepackage{fontspec}`: the line becomes a comment.
    "latex": ("core_spec.tex", "%", "\n\\usepackage{fontspec}", 1),
}


def short_dir(tag):
    """A run directory short enough for a UNIX socket path."""
    base = os.path.join(R1, "r")
    os.makedirs(base, exist_ok=True)
    n = 0
    while True:
        d = os.path.join(base, f"{tag}{n}")
        try:
            os.mkdir(d, 0o700)
            return d
        except FileExistsError:
            n += 1


def pss_kb(pid):
    try:
        with open(f"/proc/{pid}/smaps_rollup") as f:
            for line in f:
                if line.startswith("Pss:"):
                    return int(line.split()[1])
    except OSError:
        return None
    return None


def rss_kb(pid):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1])
    except OSError:
        return None
    return None


def children(pid):
    """Every child of every thread of `pid`: workers are spawned from
    the async pool's threads, not the main one."""
    out = []
    try:
        tasks = os.listdir(f"/proc/{pid}/task")
    except OSError:
        return out
    for tid in tasks:
        try:
            with open(f"/proc/{pid}/task/{tid}/children") as f:
                out.extend(int(p) for p in f.read().split())
        except OSError:
            pass
    return out


def memory(pid):
    """The daemon's PSS and RSS and every child's (worker processes)."""
    kids = children(pid)
    return {
        "daemon_pss_kb": pss_kb(pid),
        "daemon_rss_kb": rss_kb(pid),
        "children": [{"pid": k, "pss_kb": pss_kb(k), "rss_kb": rss_kb(k)} for k in kids],
        "total_pss_kb": (pss_kb(pid) or 0) + sum((pss_kb(k) or 0) for k in kids),
    }


class Daemon:
    def __init__(self, arm, init_lines, tag, extra_env=None, files=()):
        self.dir = short_dir(tag)
        for f in files:
            shutil.copy(f, self.dir)
        init_lines = [line.replace("{dir}", self.dir) for line in init_lines]
        cfg = os.path.join(self.dir, "pmacs")
        os.makedirs(cfg)
        run = os.path.join(self.dir, "run")
        os.mkdir(run, 0o700)
        init = ["pmacs.lsp.config = {}"]
        # The default theme styles none of markdown's captures but backslash
        # escapes (`punctuation` is the default style), so a markdown edit
        # shows nothing to time. Every arm runs with these markdown faces.
        init.append('pmacs.theme.merge { ["text.title"] = { fg = { 0xe0, 0xa0, 0x40 } }, '
                    '["punctuation.special"] = { fg = { 0xc0, 0x60, 0xc0 } }, '
                    '["text.emphasis"] = { fg = { 0x80, 0xc0, 0x80 } }, '
                    '["text.strong"] = { fg = { 0x80, 0xa0, 0xe0 } }, '
                    '["text.literal"] = { fg = { 0xa0, 0xa0, 0xa0 } }, '
                    '["text.reference"] = { fg = { 0x60, 0xc0, 0xc0 } }, '
                    '["text.uri"] = { fg = { 0x60, 0x90, 0xc0 } } }')
        init.append(f'pmacs.config.set("syntax.isolation", "{arm}")')
        init.extend(init_lines)
        init.append("for _, b in ipairs(pmacs.buffer.list()) do")
        init.append("  if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end")
        init.append("end")
        with open(os.path.join(cfg, "init.lua"), "w") as f:
            f.write("\n".join(init) + "\n")
        self.socket = os.path.join(self.dir, "s.sock")
        self.trace = os.path.join(self.dir, "trace.jsonl")
        env = dict(os.environ)
        for k in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME",
                  "PMACS_STATE_HOME", "HOME", "TMPDIR"):
            env[k] = self.dir
        env["XDG_RUNTIME_DIR"] = run
        env["PMACS_INSTANCE_SEMANTIC_RENDER"] = "1"
        env["PMACS_INSTANCE_MULTI_FRONTEND"] = "1"
        env["PMACS_E7I_TRACE"] = self.trace
        if extra_env:
            env.update(extra_env)
        self.stderr = open(os.path.join(self.dir, "daemon.stderr"), "w")
        self.started = time.time()
        self.proc = subprocess.Popen(
            [os.path.join(BIN, "pmacs"), "--daemon", "--socket", self.socket],
            env=env, stdout=subprocess.DEVNULL, stderr=self.stderr,
            start_new_session=True)
        deadline = time.time() + 20
        while not os.path.exists(self.socket):
            if self.proc.poll() is not None or time.time() > deadline:
                raise RuntimeError(f"daemon did not come up in {self.dir}")
            time.sleep(0.02)

    def stop(self):
        if self.proc.poll() is None:
            self.proc.send_signal(signal.SIGTERM)
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(self.proc.pid, signal.SIGKILL)
                self.proc.wait()
        # A worker the daemon did not reap is a finding, not left running.
        out = subprocess.run(["pgrep", "-f", "pmacs-parse-unit --memory-limit-mb"],
                             capture_output=True, text=True).stdout.split()
        self.stderr.close()
        return out


def probe(daemon, text, at, visible_at, samples, gap_ms=300, sample_ms=15000, deadline_ms=None,
          open_path=None):
    report = os.path.join(daemon.dir, "probe.report")
    env = dict(os.environ)
    if open_path:
        # Attach with this file as the initial target, so the probe types
        # into it whatever else the daemon holds.
        env["PMACS_GPU_PROBE_OPEN"] = open_path
    env.update({
        "PMACS_GPU_PROBE_ACTION": "latency",
        "PMACS_GPU_PROBE_TYPE_TEXT": text,
        "PMACS_GPU_PROBE_TYPE_AT": str(at),
        "PMACS_GPU_PROBE_VISIBLE_AT": str(visible_at),
        "PMACS_GPU_PROBE_SAMPLES": str(samples),
        "PMACS_GPU_PROBE_GAP_MS": str(gap_ms),
        "PMACS_GPU_PROBE_SAMPLE_MS": str(sample_ms),
        "PMACS_GPU_PROBE_DEADLINE_MS": str(deadline_ms or (samples * 4000 + 120000)),
    })
    p = subprocess.run([PROBE, "--headless-probe", daemon.socket, report],
                       env=env, capture_output=True, text=True)
    facts = {}
    if os.path.exists(report):
        with open(report) as f:
            for line in f:
                k, _, v = line.rstrip("\n").partition("=")
                facts[k] = v
    return p.returncode, facts, p.stderr


def latency(args):
    name, text, anchor, offset = FILES[args.file]
    src = os.path.join(INPUTS, name)
    data = open(src, "rb").read()
    at = data.index(anchor.encode()) + offset
    init = [
        f'pmacs.config.set("syntax.parse-memory-limit-mb", {args.unit_mb})',
        f'pmacs.config.set("syntax.parse-memory-total-mb", {args.total_mb})',
        f'pmacs.buffer.find_or_open("{{dir}}/{name}")',
    ]
    d = Daemon(args.arm, init, f"l{args.arm[0]}", files=[src])
    rc, facts, stderr, mem = None, {}, "", None
    try:
        time.sleep(args.lead_s)
        rc, facts, stderr = probe(d, text, at, at, args.samples)
        mem = memory(d.proc.pid)
    finally:
        leftovers = d.stop()
    result = {
        "kind": "latency", "impl": args.impl, "arm": args.arm, "file": args.file, "at": at, "load": os.getloadavg(),
        "text": text, "samples": args.samples, "rc": rc, "dir": d.dir,
        "visible": facts.get("visible"), "style": facts.get("style"),
        "cursor": facts.get("cursor"), "failure": facts.get("failure"),
        "memory_after": mem, "leftover_workers": leftovers,
        "unix_start": d.started,
    }
    os.makedirs(args.out, exist_ok=True)
    with open(os.path.join(args.out, "runs.jsonl"), "a") as f:
        f.write(json.dumps(result) + "\n")
    keep = os.path.join(args.out, f"{args.impl}-{args.arm}-{args.file}-{int(d.started * 1000)}")
    shutil.copytree(d.dir, keep, ignore=shutil.ignore_patterns("*.sock", name, "run"))
    shutil.rmtree(d.dir)
    print(json.dumps({k: result[k] for k in ("impl", "arm", "file", "visible", "cursor", "rc", "failure")}))
    if rc != 0 and stderr.strip():
        print(stderr[-2000:], file=sys.stderr)



import threading

VICTIMS = {"296-8k": "us-8k.md", "296-16k": "us-16k.md", "296-32k": "us-32k.md", "301": "n301.md",
           "nest-3x8k": "nest-3x8k.md"}


class Sampler(threading.Thread):
    """RSS of the daemon and its children every 100 ms, PSS every second;
    kills the daemon's group past `cap_kb` so the laptop stays usable."""

    def __init__(self, pid, cap_kb):
        super().__init__(daemon=True)
        self.pid, self.cap_kb = pid, cap_kb
        self.rows, self.stop_flag, self.capped = [], False, False
        self.t0 = time.time()

    def run(self):
        last_pss = 0
        while not self.stop_flag:
            kids = children(self.pid)
            d = rss_kb(self.pid)
            if d is None:
                break
            k = sum((rss_kb(c) or 0) for c in kids)
            row = {"t": round(time.time() - self.t0, 3), "daemon_rss_kb": d,
                   "children_rss_kb": k, "children": len(kids)}
            if time.time() - last_pss >= 1.0:
                last_pss = time.time()
                row["daemon_pss_kb"] = pss_kb(self.pid)
                row["children_pss_kb"] = sum((pss_kb(c) or 0) for c in kids)
            self.rows.append(row)
            if d + k > self.cap_kb:
                self.capped = True
                try:
                    os.killpg(self.pid, signal.SIGKILL)
                except OSError:
                    pass
                break
            time.sleep(0.1)


def trace_events(path):
    out = []
    try:
        with open(path) as f:
            for line in f:
                try:
                    out.append(json.loads(line))
                except ValueError:
                    pass
    except OSError:
        pass
    return out


def patho(args):
    kinds = args.victim.split(",")
    victims = []
    staged = os.path.join(R1, "stage")
    os.makedirs(staged, exist_ok=True)
    for i, kind in enumerate(kinds):
        dst = os.path.join(staged, f"v{i}.md")
        shutil.copy(os.path.join(INPUTS, VICTIMS[kind]), dst)
        victims.append(dst)
    name, text, anchor, offset = FILES["rust"]
    src = os.path.join(INPUTS, name)
    at = open(src, "rb").read().index(anchor.encode()) + offset
    init = [
        f'pmacs.config.set("syntax.parse-memory-limit-mb", {args.unit_mb})',
        f'pmacs.config.set("syntax.parse-memory-total-mb", {args.total_mb})',
        f'pmacs.config.set("syntax.parse-deadline-ms", {args.deadline_ms})',
        "local victims = {}",
    ] + [
        f'victims[#victims + 1] = pmacs.buffer.find_or_open("{{dir}}/v{i}.md")'
        for i in range(len(victims))
    ] + [
        f'pmacs.buffer.find_or_open("{{dir}}/{name}")',
    ]
    if not args.once:
        # Keep the victim parsing: re-dispatch every period (the wrapper
        # does nothing while one is in flight), until the run ends.
        init += [
            "pmacs.async(function()",
            "  while true do",
            "    for _, v in ipairs(victims) do",
            f"      if {str(args.touch).lower()} then pcall(function() v:insert(v:len(), ' ') end) end",
            "      pcall(pmacs.parse._dispatch, v, 'markdown')",
            "    end",
            f"    pmacs.workers.sleep({args.period_ms}):await()",
            "  end",
            "end)",
        ]
    d = Daemon(args.arm, init, f"p{args.arm[0]}", files=[src] + victims)
    sampler = Sampler(d.proc.pid, args.cap_mb * 1024)
    sampler.start()
    rc, facts, stderr = None, {}, ""
    try:
        time.sleep(args.lead_s)
        rc, facts, stderr = probe(d, text, at, at, args.samples, sample_ms=20000,
                                  open_path=os.path.join(d.dir, name))
        time.sleep(args.tail_s)
        alive = d.proc.poll() is None
    finally:
        sampler.stop_flag = True
        sampler.join()
        leftovers = d.stop()
    events = trace_events(d.trace)
    deaths = [e for e in events if e.get("event") == "death"]
    rows = sampler.rows
    peak = max(((r["daemon_rss_kb"] + r["children_rss_kb"]) for r in rows), default=0)
    result = {
        "kind": "patho", "impl": args.impl, "arm": args.arm, "victim": args.victim, "rc": rc, "once": args.once,
        "touch": args.touch, "load": os.getloadavg(),
        "alive_at_end": alive, "capped": sampler.capped,
        "visible": facts.get("visible"), "cursor": facts.get("cursor"),
        "failure": facts.get("failure"), "samples": args.samples,
        "unit_mb": args.unit_mb, "total_mb": args.total_mb, "deadline_ms": args.deadline_ms,
        "deaths": len(deaths),
        "death_kinds": sorted({e.get("kind") for e in deaths}),
        "death_call_ms": [round(e.get("call_us", 0) / 1000) for e in deaths],
        "peak_rss_kb": peak,
        "first_rss_kb": rows[0]["daemon_rss_kb"] if rows else None,
        "last_rss_kb": (rows[-1]["daemon_rss_kb"] + rows[-1]["children_rss_kb"]) if rows else None,
        "leftover_workers": leftovers, "dir": d.dir, "unix_start": d.started,
    }
    os.makedirs(args.out, exist_ok=True)
    tag = args.victim if len(kinds) == 1 else f"{len(kinds)}x{kinds[0]}"
    keep = os.path.join(args.out, f"patho-{args.impl}-{args.arm}-{tag}-{int(d.started * 1000)}")
    shutil.copytree(d.dir, keep, ignore=shutil.ignore_patterns("*.sock", "run", name, "v*.md"))
    with open(os.path.join(keep, "memory.jsonl"), "w") as f:
        for r in rows:
            f.write(json.dumps(r) + "\n")
    shutil.rmtree(d.dir)
    with open(os.path.join(args.out, "runs.jsonl"), "a") as f:
        f.write(json.dumps(result) + "\n")
    print(json.dumps({k: result[k] for k in ("impl", "arm", "victim", "alive_at_end", "capped", "visible",
                                              "deaths", "death_kinds", "death_call_ms",
                                              "peak_rss_kb", "last_rss_kb", "failure")}))
    if rc != 0 and stderr.strip():
        print(stderr[-2000:], file=sys.stderr)



SET = ["Cargo.toml", "ci.yml", "drv.c", "gate.sh", "pty_probe.py", "README.md",
       "semantic_render.rs", "syntax.lua"]


def steady(args):
    """Eleven buffers in nine languages open, `editor.rs` typed into, then
    the memory of the daemon and every unit it owns, as PSS."""
    name, text, anchor, offset = FILES["rust"]
    src = os.path.join(INPUTS, name)
    at = open(src, "rb").read().index(anchor.encode()) + offset
    others = [os.path.join(INPUTS, "set", f) for f in SET] + [
        os.path.join(INPUTS, "roadmap.md"), os.path.join(INPUTS, "core_spec.tex")]
    init = [
        f'pmacs.config.set("syntax.parse-memory-limit-mb", {args.unit_mb})',
        f'pmacs.config.set("syntax.parse-memory-total-mb", {args.total_mb})',
    ] + [f'pmacs.config.set("{k}", {v})' for k, v in (kv.split("=", 1) for kv in (args.set or []))] + [
        f'pmacs.buffer.find_or_open("{{dir}}/{os.path.basename(o)}")' for o in others] + [
        f'pmacs.buffer.find_or_open("{{dir}}/{name}")',
    ]
    extra = dict(kv.split("=", 1) for kv in args.env) if args.env else None
    d = Daemon(args.arm, init, f"s{args.arm[0]}", files=[src] + others, extra_env=extra)
    rc, facts, before, after = None, {}, None, None
    try:
        time.sleep(args.lead_s)
        before = memory(d.proc.pid)
        rc, facts, _ = probe(d, text, at, at, args.samples, open_path=os.path.join(d.dir, name))
        time.sleep(2)
        after = memory(d.proc.pid)
        # The mappings behind the figure, for the record.
        for pid, tag in [(d.proc.pid, "daemon")] + [(c, f"child-{c}") for c in children(d.proc.pid)[:2]]:
            try:
                shutil.copy(f"/proc/{pid}/smaps", os.path.join(d.dir, f"smaps-{tag}.txt"))
            except OSError:
                pass
    finally:
        leftovers = d.stop()
    result = {
        "kind": "steady", "impl": args.impl, "arm": args.arm, "env": args.env, "set": args.set, "rc": rc,
        "load": os.getloadavg(),
        "buffers": len(others) + 1,
        "visible": facts.get("visible"), "failure": facts.get("failure"),
        "memory_settled": before, "memory_after_typing": after,
        "leftover_workers": leftovers, "unix_start": d.started,
    }
    keep = os.path.join(args.out, f"steady-{args.impl}-{args.arm}-{int(d.started * 1000)}")
    os.makedirs(args.out, exist_ok=True)
    shutil.copytree(d.dir, keep, ignore=shutil.ignore_patterns(
        "*.sock", "run", name, *[os.path.basename(o) for o in others]))
    shutil.rmtree(d.dir)
    with open(os.path.join(args.out, "runs.jsonl"), "a") as f:
        f.write(json.dumps(result) + "\n")
    print(json.dumps({"impl": args.impl, "arm": args.arm, "visible": result["visible"],
                      "settled_total_pss_mb": round((before or {}).get("total_pss_kb", 0) / 1024, 1),
                      "after_total_pss_mb": round((after or {}).get("total_pss_kb", 0) / 1024, 1),
                      "daemon_pss_mb": round(((after or {}).get("daemon_pss_kb") or 0) / 1024, 1),
                      "workers": len((after or {}).get("children", []))}))


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    lat = sub.add_parser("latency")
    lat.add_argument("--arm", required=True, choices=["none", "process"])
    lat.add_argument("--file", required=True, choices=list(FILES))
    lat.add_argument("--samples", type=int, default=30)
    lat.add_argument("--unit-mb", type=int, default=1024)
    lat.add_argument("--total-mb", type=int, default=4096)
    lat.add_argument("--lead-s", type=float, default=4.0)
    lat.add_argument("--out", required=True)
    lat.add_argument("--impl", required=True, choices=list(IMPLS))
    pat = sub.add_parser("patho")
    pat.add_argument("--arm", required=True, choices=["none", "process"])
    pat.add_argument("--victim", required=True, help="one of VICTIMS, or a comma list of them")
    pat.add_argument("--samples", type=int, default=20)
    pat.add_argument("--unit-mb", type=int, default=1024)
    pat.add_argument("--total-mb", type=int, default=4096)
    pat.add_argument("--deadline-ms", type=int, default=5000)
    pat.add_argument("--period-ms", type=int, default=1000)
    pat.add_argument("--lead-s", type=float, default=2.0)
    pat.add_argument("--tail-s", type=float, default=2.0)
    pat.add_argument("--cap-mb", type=int, default=12288)
    pat.add_argument("--touch", action="store_true",
                     help="append a byte to each victim before each re-dispatch, so every arm reparses")
    pat.add_argument("--once", action="store_true",
                     help="parse the victim once, at load, and never again")
    pat.add_argument("--out", required=True)
    pat.add_argument("--impl", required=True, choices=list(IMPLS))
    st = sub.add_parser("steady")
    st.add_argument("--arm", required=True, choices=["none", "process"])
    st.add_argument("--samples", type=int, default=20)
    st.add_argument("--unit-mb", type=int, default=1024)
    st.add_argument("--total-mb", type=int, default=4096)
    st.add_argument("--lead-s", type=float, default=15.0)
    st.add_argument("--env", action="append", help="KEY=VALUE for the daemon (a variant run)")
    st.add_argument("--set", action="append", help="pmacs.config KEY=LUA_VALUE (a variant run)")
    st.add_argument("--out", required=True)
    st.add_argument("--impl", required=True, choices=list(IMPLS))
    args = ap.parse_args()
    global BIN
    BIN = IMPLS[args.impl]
    if args.cmd == "steady":
        steady(args)
    if args.cmd == "latency":
        latency(args)
    elif args.cmd == "patho":
        patho(args)


if __name__ == "__main__":
    main()
