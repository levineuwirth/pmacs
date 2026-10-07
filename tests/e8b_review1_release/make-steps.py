#!/usr/bin/env python3
"""E8b review 1: one key sequence, written for both drivers.

make-steps.py writes steps-tui.json (tui_drive.py) and gpu.script
(run-gpu.sh) beside itself from the list below, so the TUI and the GPU
run the same keys. Each `switch` is `C-x b <text> RET`, then the band
captured, then `C-c z` (session.sh's `review.where`) and the band
captured again: what the switch said, and what the window shows.
"""
import json, os

HERE = os.path.dirname(os.path.abspath(__file__))

SEQ = [
    ("where", "start"),
    ("switch", "b.rs", "bare"),            # E8b.1's bare name, from a.rs
    ("switch", "b.rs", "bare-again"),      # E8b.2's "already showing"
    ("switch", "nts", "nts"),              # D18: notes.txt
    ("switch", "scr", "scr"),              # D18: *scratch*
    ("switch", "zzz", "zzz"),              # D18: refused as typed
    ("switch", "editor.rs", "editor"),
    ("dired", "dired-open"),               # C-x d RET: dired on proj
    ("switch", "editor.rs", "editor-2"),
    ("switch", "dired", "dired"),          # the dired buffer, or a file?
    ("switch", "editor.rs", "editor-3"),
    ("switch", "dir", "dir"),
    ("switch", "*dir", "star-dir"),
    ("switch", "scr", "scr-2"),
    ("kill", "dired", 9, "kill-dired"),    # C-x k from *scratch* (9 chars)
    ("switch", "dired.lua", "after-kill"),
    ("lsp", "lsp-open"),                   # C-c l opens *lsp*
    ("switch", "scr", "scr-3"),
    ("switch", "lsp", "lsp"),              # D18: *lsp*, src/lsp.rs open
]

tui, gpu = [], []


def where(label):
    tui.extend([["keys", "C-c", "z"], ["capture", label + "-where"]])
    gpu.extend(["key:C-c", "key:z", f"report:{label}-where"])


for step in SEQ:
    kind = step[0]
    if kind == "where":
        where(step[1])
    elif kind == "switch":
        _, text, label = step
        tui.extend([["keys", "C-x", "b"], ["text", text], ["keys", "Enter"], ["capture", label]])
        gpu.extend(["key:C-x", "key:b"] + [f"key:{c}" for c in text] + ["key:ret", f"report:{label}"])
        where(label)
    elif kind == "dired":
        tui.extend([["keys", "C-x", "d"], ["sleep", 0.5], ["keys", "Enter"], ["sleep", 1.5],
                    ["capture", step[1]]])
        gpu.extend(["key:C-x", "key:d", "key:ret", "quiet:1500", f"report:{step[1]}"])
        where(step[1])
    elif kind == "kill":
        _, text, prefill, label = step
        tui.extend([["keys", "C-x", "k", "C-a"] + ["C-d"] * prefill, ["text", text],
                    ["keys", "Enter"], ["capture", label]])
        gpu.extend(["key:C-x", "key:k", "key:C-a"] + ["key:C-d"] * prefill
                   + [f"key:{c}" for c in text] + ["key:ret", f"report:{label}"])
        where(label)
    elif kind == "lsp":
        tui.extend([["keys", "C-c", "l"], ["sleep", 0.5], ["capture", step[1]]])
        gpu.extend(["key:C-c", "key:l", "quiet:500", f"report:{step[1]}"])
        where(step[1])

json.dump(tui, open(f"{HERE}/steps-tui.json", "w"), indent=0)
open(f"{HERE}/gpu.script", "w").write("\n".join(gpu) + "\n")
print(len(tui), "tui steps,", len(gpu), "gpu steps")
