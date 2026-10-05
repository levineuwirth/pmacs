#!/usr/bin/env python3
"""marked.py <capture-esc.txt>...: each capture's popup row (the line
holding the grid's active-parameter mark, SGR 1;4) with the marked text,
and the status line's caret, from tmux capture-pane -e output."""
import re, sys
for path in sys.argv[1:]:
    text = open(path, encoding="utf-8").read()
    rows = text.split("\n")
    for r in rows:
        for m in re.finditer(r"\x1b\[1;4m([^\x1b]*)\x1b\[0m", r):
            plain = re.sub(r"\x1b\[[0-9;]*m", "", r)
            print(f"{path.rsplit('/',1)[-1]}: marked {m.group(1)!r} in {plain.strip()!r}")
    caret = [re.sub(r"\x1b\[[0-9;]*m", "", r) for r in rows if re.search(r"L\d+:C\d+", re.sub(r"\x1b\[[0-9;]*m", "", r))]
    if caret:
        print(f"{path.rsplit('/',1)[-1]}: status {re.search(r'L\d+:C\d+', caret[-1]).group(0)}")
