#!/usr/bin/env python3
"""Reproduce C6b launch/typing observations with isolated daemon roots.

Usage: python3 tests/probes/e6b_closure.py BINARY_DIR OUTPUT_DIR [cases|launch]
The launch arm opens real GPU windows for thirty seconds per arm.
"""
import collections
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]


def environment(root):
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    env = os.environ.copy()
    for key in ('XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'XDG_CACHE_HOME', 'PMACS_STATE_HOME'):
        env[key] = str(root)
    env['PMACS_C6B_REVIEW_RUN'] = str(root)
    env['PMACS_GPU_DEBUG_APPLY'] = '1'
    return env


def stop(root, process=None):
    if process is not None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    if not Path('/proc').is_dir():
        return
    marker = ('PMACS_C6B_REVIEW_RUN=' + str(root)).encode()
    owned = []
    for entry in Path('/proc').iterdir():
        if entry.name.isdigit():
            try:
                if marker in (entry / 'environ').read_bytes().split(b'\0'):
                    owned.append(int(entry.name))
            except (OSError, PermissionError):
                pass
    for pid in owned:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass


def config(root, text, binary, extra=''):
    (root / 'pmacs').mkdir(exist_ok=True)
    fixture = root / 'a.rs'
    fixture.write_text(text)
    proxy = REPO / 'tests/probes/e6b_lsp_proxy.py'
    init = f'''pmacs.lsp.config.rust = {{ command = 'python3',
 args = {{ {json.dumps(str(proxy))}, {json.dumps(str(root / 'lsp.jsonl'))}, {json.dumps(str(binary / 'pmacs_fake_lsp'))} }},
 env = {{ PMACS_FAKE_LSP_MODE = 'semantichold', PMACS_FAKE_LSP_SEMANTIC_HOLD_MS = '400' }} }}
pmacs.theme.merge {{ namespace = {{ fg = {{ 0x7b, 0x1f, 0xa2 }} }} }}
pmacs.buffer.find_or_open({json.dumps(str(fixture))})
{extra}
'''
    (root / 'pmacs/init.lua').write_text(init)
    return fixture


def daemon(binary, root, env):
    err = open(root / 'daemon.stderr', 'w')
    process = subprocess.Popen([str(binary / 'pmacs'), '--daemon', '--socket', str(root / 's.sock')], env=env,
                               stdout=subprocess.DEVNULL, stderr=err, start_new_session=True)
    deadline = time.monotonic() + 15
    while not (root / 's.sock').exists():
        if process.poll() is not None or time.monotonic() > deadline:
            stop(root, process)
            process.wait(timeout=10)
            raise RuntimeError('daemon failed: ' + (root / 'daemon.stderr').read_text())
        time.sleep(.02)
    return process


def probe(binary, root, env, name, at, text, action='', select=0):
    env = dict(env, PMACS_GPU_PROBE_TYPE_TEXT=text, PMACS_GPU_PROBE_TYPE_AT=str(at),
               PMACS_GPU_PROBE_ACTION=action, PMACS_GPU_PROBE_SELECT_BYTES=str(select),
               PMACS_GPU_PROBE_SETTLE='change', PMACS_GPU_PROBE_OBSERVE_MS='1800')
    with open(root / (name + '.stderr'), 'w') as err:
        result = subprocess.run([str(binary / 'pmacs-gpu'), '--headless-probe', str(root / 's.sock'), str(root / (name + '.txt'))],
                                env=env, stdout=subprocess.DEVNULL, stderr=err, timeout=45)
    if result.returncode == 3:
        raise SystemExit(3)
    if result.returncode:
        raise RuntimeError((root / (name + '.stderr')).read_text())
    report = (root / (name + '.txt')).read_text()
    print(name, report, flush=True)
    return report


def validate_case(name, report):
    facts = dict(line.split('=', 1) for line in report.splitlines() if '=' in line)
    frames = [facts[f'frame.{i}'].split('|') for i in range(int(facts['frames']))]
    key = next(i for i, f in enumerate(frames) if f[1] == 'key')
    def marked(frame):
        return [tuple(map(int, r.split('-'))) for r in frame[3].split(';') if r]
    assert facts['completion_observed'] == 'true'
    for frame in frames[key:]:
        assert (0, 2) in marked(frame), ('fn lost its refinement', frame)
    expected = {
        'paren': ('fn main(() {}\n', [(0, 2), (3, 7)]),
        'word': ('fn main() word{ a; }\n', [(0, 2), (3, 7), (10, 14), (16, 17)]),
        'paste': ('fn main() { alXYeta; }\n', [(0, 2), (3, 7), (12, 19)]),
        'dot': ('fn main() { ab; }\n', [(0, 2), (3, 7), (12, 14)]),
    }
    text, final_spans = expected[name]
    assert json.loads(facts['final_text_debug']) == text
    assert marked(frames[-1]) == final_spans, (name, frames[-1])
    styles = [marked(f) for f in frames[key+1:] if f[1] == 'StyleSpans']
    # Characterize the reported intermediate states, without pretending
    # the first changed frame is necessarily the semantic answer.
    if name == 'paren':
        assert (3, 8) in styles[0]
    elif name == 'word':
        assert (10, 14) not in styles[0]
    elif name == 'paste':
        echoed = next(f for f in frames[key+1:] if f[1] == 'CrdtOp')
        assert (12, 16) in marked(echoed)
        assert (12, 14) in styles[0]
    elif name == 'dot':
        assert (12, 13) in marked(frames[key]) and (12, 13) in styles[0]


def cases(binary, output):
    scenarios = [('paren', 'fn main() {}\n', 7, '(', '', 0),
                 ('word', 'fn main() { a; }\n', 10, 'word', '', 0),
                 ('paste', 'fn main() { alpha.beta; }\n', 14, 'XY', 'paste', 5),
                 ('dot', 'fn main() { a.b; }\n', 13, '_', 'delete', 0)]
    for name, text, at, value, action, select in scenarios:
        root = output / name
        env = environment(root)
        env['PMACS_C6B_DIFFERENT_FACES'] = '1'
        config(root, text, binary, "for _,b in ipairs(pmacs.buffer.list()) do if b:name() == '*scratch*' then pmacs.buffer.kill(b) end end")
        process = daemon(binary, root, env)
        try:
            report = probe(binary, root, env, name, at, value, action, select)
            validate_case(name, report)
        finally:
            stop(root, process)
            process.wait(timeout=10)


def launch(binary, output):
    for with_target in (True, False):
        root = output / ('target' if with_target else 'no-target')
        env = environment(root)
        (root / 'pmacs').mkdir(exist_ok=True)
        fixture = root / 'a.rs'
        fixture.write_text('fn main() {}\n')
        # Both arms have scratch plus the file; only initial_target differs.
        (root / 'pmacs/init.lua').write_text('pmacs.lsp.config = {}\npmacs.buffer.find_or_open(' + json.dumps(str(fixture)) + ')\n')
        command = [str(binary / 'pmacs'), '--gpu', '--socket', str(root / 's.sock')]
        if with_target:
            command.append(str(fixture))
        try:
            with open(root / 'launch.stderr', 'w') as err:
                process = subprocess.Popen(command, env=env, stderr=err, stdout=subprocess.DEVNULL, start_new_session=True)
                try:
                    process.wait(timeout=30)
                    raise RuntimeError('window exited before thirty seconds')
                except subprocess.TimeoutExpired:
                    pass
        finally:
            stop(root, process)
            process.wait(timeout=10)
        lines = (root / 'launch.stderr').read_text().splitlines()
        counts = collections.Counter(line for line in lines if 'BufferSnapshot' in line)
        result = {'initial_target': with_target, 'seconds': 30, 'snapshots': sum(counts.values()), 'examples': list(counts)[:4]}
        (root / 'count.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result), flush=True)


if __name__ == '__main__':
    binaries, output, mode = sys.argv[1:]
    {'cases': cases, 'launch': launch}[mode](Path(binaries).resolve(), Path(output).resolve())
