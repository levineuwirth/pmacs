#!/usr/bin/env python3
"""Interleave warm range/no-range arms at the identical byte in editor.rs.

The no-range arm overrides the private viewport query in isolated init.lua;
it returns nil, so the production pull uses its existing whole-file fallback.
No source or on-disk editor.rs edit is made. Reset keystrokes are not samples.
"""
import hashlib
import json
from pathlib import Path
import sys
from e6b_closure import REPO, daemon, environment, probe, stop


def parse(report):
    facts = dict(line.split('=', 1) for line in report.splitlines() if '=' in line)
    frames = [facts[f'frame.{i}'].split('|') for i in range(int(facts['frames']))]
    key = next(i for i, f in enumerate(frames) if f[1] == 'key')
    at = int(frames[key][0])
    post = next(f for f in frames[key + 1:] if f[1] == 'StyleSpans')
    # Stale std lives at [706,709); rust-analyzer's xstd answer removes it.
    def carries_std(f):
        return any(int(r.split('-')[0]) <= 706 and int(r.split('-')[1]) >= 709 for r in f[3].split(';') if r)
    assert carries_std(frames[key]), 'keystroke has no stale std refinement'
    answer = next(f for f in frames[key + 1:] if f[1] == 'StyleSpans' and not carries_std(f))
    return {'grammar_ms': int(post[0]) - at, 'answer_ms': int(answer[0]) - at,
            'reported_window_ms': facts['window_ms']}


def main():
    binary, output, samples = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve(), int(sys.argv[3])
    source = REPO / 'src/editor.rs'
    original = source.read_bytes()
    assert original[705:708] == b'std'
    roots, processes = {}, []
    try:
        for arm in ('full', 'range'):
            root = output / arm
            env = environment(root)
            # Both rust-analyzers are configured identically, including inlays.
            (root / 'pmacs').mkdir(exist_ok=True)
            proxy = REPO / 'tests/probes/e6b_lsp_proxy.py'
            wrappers = '''
local timing_path = TIMING_PATH
local function wrap(name)
 local original = pmacs.lsp[name]
 if type(original) ~= 'function' then return end
 pmacs.lsp[name] = function(...)
  local before = pmacs.editor.monotonic_ms()
  local result = original(...)
  local after = pmacs.editor.monotonic_ms()
  local f = assert(io.open(timing_path, 'a'))
  f:write(name .. ',' .. before .. ',' .. after .. '\\n'); f:close()
  return result
 end
end
for _, name in ipairs({'_visible_lines', 'did_change', 'request_inlay_hint',
 'request_semantic_tokens_range', 'request_semantic_tokens', 'request_semantic_tokens_delta'}) do wrap(name) end
'''.replace('TIMING_PATH', json.dumps(str(root / 'timings.csv')))
            disable = "pmacs.lsp._visible_lines = function() return nil end\n" if arm == 'full' else ''
            init = f'''pmacs.lsp.config.rust = {{ command = 'python3', args = {{ {json.dumps(str(proxy))}, {json.dumps(str(root / 'lsp.jsonl'))}, 'rust-analyzer' }} }}
pmacs.theme.merge {{ namespace = {{ fg = {{ 0x7b, 0x1f, 0xa2 }} }}, property = {{ fg = {{ 0x7b, 0x1f, 0xa2 }} }} }}
{disable}{wrappers}
pmacs.buffer.find_or_open({json.dumps(str(source))})
for _,b in ipairs(pmacs.buffer.list()) do if b:name() == '*scratch*' then pmacs.buffer.kill(b) end end
'''
            (root / 'pmacs/init.lua').write_text(init)
            processes.append(daemon(binary, root, env))
            roots[arm] = (root, env)
        # Explicit warmups in both processes; no first/cold sample in the result.
        for arm in ('full', 'range'):
            root, env = roots[arm]
            probe(binary, root, env, 'warmup', 705, 'x')
            probe(binary, root, env, 'warmup-reset', 705, '_', 'delete')
        for arm in ('full', 'range'):
            root, _ = roots[arm]
            assert (root / 'timings.csv').exists(), 'timing wrapper did not complete'
            requests = [json.loads(l)['message'].get('method') for l in (root / 'lsp.jsonl').read_text().splitlines()]
            assert ('textDocument/semanticTokens/range' in requests) == (arm == 'range'), 'arms did not differ on the wire'
        results = []
        for i in range(samples):
            order = ('full', 'range') if i % 2 == 0 else ('range', 'full')
            for arm in order:
                root, env = roots[arm]
                report = probe(binary, root, env, f'sample-{i:02d}', 705, 'x')
                row = dict(arm=arm, sample=i, **parse(report))
                results.append(row)
                (output / 'samples.json').write_text(json.dumps(results, indent=2) + '\n')
                print('SAMPLE', json.dumps(row), flush=True)
                reset = probe(binary, root, env, f'reset-{i:02d}', 705, '_', 'delete')
                # Reset must restore exactly the original text; parse JSON escaped Rust string.
                final = next(l.split('=', 1)[1] for l in reset.splitlines() if l.startswith('final_text_debug='))
                assert json.loads(final).encode() == original, 'reset text drifted'
        (output / 'source.sha256').write_text(hashlib.sha256(original).hexdigest() + '\n')
    finally:
        for (root, _), process in zip(roots.values(), processes):
            stop(root, process)
        for process in processes:
            process.wait(timeout=10)
        assert source.read_bytes() == original


if __name__ == '__main__':
    main()
