#!/usr/bin/env python3
"""Record actual LSP frames in both directions; preserve framing and payloads."""
import json
import os
import subprocess
import sys
import threading
import time


def frame(stream):
    headers = {}
    while True:
        line = stream.readline()
        if not line:
            return None
        if line in (b"\r\n", b"\n"):
            break
        k, v = line.decode().split(":", 1)
        headers[k.lower()] = v.strip()
    return stream.read(int(headers["content-length"]))


def main():
    sink, *command = sys.argv[1:]
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    lock = threading.Lock()
    documents = {}
    anchors = {}
    distinct = os.environ.get('PMACS_C6B_DIFFERENT_FACES') == '1' 
    with open(sink, "a", buffering=1) as log:
        def copy(source, target, direction):
            while (body := frame(source)) is not None:
                message = json.loads(body)
                if distinct:
                    method = message.get('method', '')
                    params = message.get('params', {})
                    if direction == 'request':
                        doc = params.get('textDocument', {})
                        uri = doc.get('uri')
                        if method == 'textDocument/didOpen':
                            documents[uri] = doc['text']
                        if method == 'textDocument/didChange':
                            documents[uri] = params['contentChanges'][-1]['text']
                        if method.startswith('textDocument/semanticTokens/'):
                            anchors[message['id']] = documents.get(uri, '')
                    else:
                        result = message.get('result', {})
                        if isinstance(result, dict):
                            provider = result.get('capabilities', {}).get('semanticTokensProvider')
                            if provider:
                                provider['legend']['tokenTypes'] = ['namespace', 'property']
                            if 'data' in result and message.get('id') in anchors:
                                lines = anchors[message['id']].splitlines()
                                line = column = 0
                                data = result['data']
                                for i in range(0, len(data), 5):
                                    dl, dc, length = data[i:i+3]
                                    line += dl
                                    column = dc if dl else column + dc
                                    word = lines[line][column:column+length]
                                    data[i+3] = int(word in ('b', 'beta'))
                    body = json.dumps(message).encode()
                record = {"ns": time.monotonic_ns(), "unix_ms": time.time_ns() // 1_000_000, "direction": direction,
                          "message": message}
                with lock:
                    log.write(json.dumps(record) + "\n")
                target.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
                target.flush()
        inbound = threading.Thread(target=copy, args=(sys.stdin.buffer, child.stdin, "request"), daemon=True)
        inbound.start()
        try:
            copy(child.stdout, sys.stdout.buffer, "response")
        finally:
            child.terminate()
            child.wait()


if __name__ == "__main__":
    main()
