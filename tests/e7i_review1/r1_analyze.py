#!/usr/bin/env python3
"""E7i review 1: tables from a r1_campaign.py run, in the comparison's terms.
Pooled samples per cell; per-round p50; the comparison's tie band per
quantile per file (the largest of 2 ms, 10% of native's value and the spread
between native batches in the same run), native being the built tip's
`none` arm."""
import json, os, statistics, sys

OUT = sys.argv[1]
runs = [json.loads(l) for l in open(os.path.join(OUT, "runs.jsonl"))]


def q(values, p):
    if not values:
        return None
    s = sorted(values)
    k = (len(s) - 1) * p
    lo, hi = int(k), min(int(k) + 1, len(s) - 1)
    return s[lo] + (s[hi] - s[lo]) * (k - lo)


def samples(run_dir, key="visible"):
    out = []
    try:
        for line in open(os.path.join(run_dir, "probe.report")):
            if line.startswith("sample."):
                for kv in line.split("=", 1)[1].split():
                    k, _, v = kv.partition("=")
                    if k == key and v not in ("none", ""):
                        try:
                            out.append(float(v))
                        except ValueError:
                            pass
    except OSError:
        pass
    return out


def keep_dir(r):
    base = os.path.basename(r["dir"])
    for name in os.listdir(OUT):
        if name.startswith(f'{r["impl"]}-{r["arm"]}-{r.get("file", "")}-') or name.startswith(
                f'patho-{r["impl"]}-{r["arm"]}-') or name.startswith(f'steady-{r["impl"]}-{r["arm"]}-'):
            if name.endswith(str(int(r["unix_start"] * 1000))):
                return os.path.join(OUT, name)
    return None


print("## Keystroke to visible update (ms), pooled samples\n")
print("| file | build | arm | n | p50 | p95 | p99 | per-round p50 | loads |")
print("|---|---|---|---|---|---|---|---|---|")
cells = {}
for f in ["rust", "markdown", "latex"]:
    for impl, arm in [("built", "none"), ("built", "process"), ("proto", "process"), ("proto", "none")]:
        rs = [r for r in runs if r["kind"] == "latency" and r["file"] == f and r["impl"] == impl and r["arm"] == arm]
        if not rs:
            continue
        dirs = [keep_dir(r) for r in rs]
        per = [samples(d) for d in dirs if d]
        pooled = [x for s in per for x in s]
        cells[(f, impl, arm)] = {"pooled": pooled, "per": per}
        print(f"| {f} | {impl} | {arm} | {len(pooled)} | {q(pooled, .5):.0f} | {q(pooled, .95):.0f} | "
              f"{q(pooled, .99):.0f} | {', '.join(f'{q(s, .5):.0f}' for s in per if s)} | "
              f"{', '.join(f'{r['load'][0]:.1f}' for r in rs)} |")

print("\n## Against native (built none), by the comparison's band\n")
print("| file | arm | quantile | value | native | diff | band | verdict |")
print("|---|---|---|---|---|---|---|---|")
for f in ["rust", "markdown"]:
    nat = cells.get((f, "built", "none"))
    if not nat:
        continue
    for impl, arm in [("built", "process"), ("proto", "process"), ("proto", "none")]:
        c = cells.get((f, impl, arm))
        if not c:
            continue
        for p in (.5, .95, .99):
            nv = q(nat["pooled"], p)
            batch = [q(s, p) for s in nat["per"] if s]
            band = max(2.0, 0.1 * nv, (max(batch) - min(batch)) if batch else 0)
            v = q(c["pooled"], p)
            d = v - nv
            print(f"| {f} | {impl} {arm} | p{int(p*100)} | {v:.0f} | {nv:.0f} | {d:+.0f} | {band:.0f} | "
                  f"{'inside' if abs(d) <= band else ('HIGHER' if d > 0 else 'lower')} |")

print("\n## Paired by round: built process minus proto process, and built process minus built none (p50, ms)\n")
for f in ["rust", "markdown"]:
    a = cells.get((f, "built", "process"), {}).get("per", [])
    b = cells.get((f, "proto", "process"), {}).get("per", [])
    n = cells.get((f, "built", "none"), {}).get("per", [])
    d1 = [q(x, .5) - q(y, .5) for x, y in zip(a, b) if x and y]
    d2 = [q(x, .5) - q(y, .5) for x, y in zip(a, n) if x and y]
    print(f"- {f}: built-proto {', '.join(f'{d:+.0f}' for d in d1)}; built-native {', '.join(f'{d:+.0f}' for d in d2)}")

print("\n## Responsiveness during a pathological parse\n")
print("| victim | build | arm | n | typed p50 | p99 | caret p50 | p99 | deaths (kinds) | peak MB | last MB | alive |")
print("|---|---|---|---|---|---|---|---|---|---|---|---|")
for v in ["296-16k", "301"]:
    for impl, arm in [("built", "none"), ("built", "process"), ("proto", "process")]:
        rs = [r for r in runs if r["kind"] == "patho" and not r["once"] and r["victim"] == v
              and r["impl"] == impl and r["arm"] == arm]
        if not rs:
            continue
        dirs = [keep_dir(r) for r in rs]
        vis = [x for d in dirs if d for x in samples(d)]
        car = [x for d in dirs if d for x in samples(d, "cursor")]
        deaths = sum(r["deaths"] for r in rs)
        kinds = sorted({k for r in rs for k in r["death_kinds"] if k})
        print(f"| {v} | {impl} | {arm} | {len(vis)} | {q(vis, .5) or 0:.0f} | {q(vis, .99) or 0:.0f} | "
              f"{q(car, .5) or 0:.0f} | {q(car, .99) or 0:.0f} | {deaths} ({','.join(kinds)}) | "
              f"{max(r['peak_rss_kb'] for r in rs)/1024:.0f} | {max((r['last_rss_kb'] or 0) for r in rs)/1024:.0f} | "
              f"{all(r['alive_at_end'] for r in rs)} |")

print("\n## Steady state: eleven buffers, PSS after typing (MB)\n")
print("| build | arm | runs | total PSS each | median | daemon PSS | workers |")
print("|---|---|---|---|---|---|---|")
for impl, arm in [("built", "none"), ("built", "process"), ("proto", "process")]:
    rs = [r for r in runs if r["kind"] == "steady" and r["impl"] == impl and r["arm"] == arm and r["memory_after_typing"]]
    if not rs:
        continue
    tot = [r["memory_after_typing"]["total_pss_kb"] / 1024 for r in rs]
    dae = [(r["memory_after_typing"]["daemon_pss_kb"] or 0) / 1024 for r in rs]
    wk = [len(r["memory_after_typing"]["children"]) for r in rs]
    print(f"| {impl} | {arm} | {len(rs)} | {', '.join(f'{t:.1f}' for t in tot)} | {statistics.median(tot):.1f} | "
          f"{', '.join(f'{d:.1f}' for d in dae)} | {', '.join(map(str, wk))} |")

print("\n## Acceptance, once, the built tip in process mode\n")
for r in runs:
    if r["kind"] == "patho" and r["once"]:
        print(f"- {r['victim']}: deaths {r['deaths']} {r['death_kinds']} at {r['death_call_ms']} ms; peak "
              f"{r['peak_rss_kb']/1024:.0f} MB, last {(r['last_rss_kb'] or 0)/1024:.0f} MB; typed visible "
              f"{r['visible']}; alive {r['alive_at_end']}; leftover workers {r['leftover_workers']}")

print("\n## In-unit parse and the boundary, per keystroke (ms), the open's cold parse excluded\n")
print("| file | build | arm | parses | in-unit p50 | p95 | boundary p50 (call - unit) |")
print("|---|---|---|---|---|---|---|")
for f in ["rust", "markdown"]:
    for impl, arm in [("built", "none"), ("built", "process"), ("proto", "process"), ("proto", "none")]:
        rs = [r for r in runs if r["kind"] == "latency" and r["file"] == f and r["impl"] == impl and r["arm"] == arm]
        unit, boundary = [], []
        for r in rs:
            d = keep_dir(r)
            if not d:
                continue
            first = True
            for line in open(os.path.join(d, "trace.jsonl")):
                try:
                    e = json.loads(line)
                except ValueError:
                    continue
                if e.get("event") != "parsed":
                    continue
                if first:
                    first = False
                    continue
                unit.append(e["unit_us"] / 1000)
                if "call_us" in e:
                    boundary.append((e["call_us"] - e["unit_us"]) / 1000)
        if unit:
            b = f"{q(boundary, .5):.2f}" if boundary else "-"
            print(f"| {f} | {impl} | {arm} | {len(unit)} | {q(unit, .5):.1f} | {q(unit, .95):.1f} | {b} |")
