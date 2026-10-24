#!/usr/bin/env python3
"""Pack atutop's browser version (tui/dist, built with `trunk build --release`)
into atu100_bridge/web_files.h: each file gzipped, as a C array the bridge
serves with Content-Encoding: gzip. Run it after every trunk build, before
compiling the bridge.

    esp32/make_web_files.py [DIST_DIR]
"""
import gzip
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DIST = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, '../tui/dist')
OUT = os.path.join(HERE, 'atu100_bridge/web_files.h')
TYPES = {'.html': 'text/html; charset=utf-8', '.js': 'application/javascript',
         '.wasm': 'application/wasm', '.css': 'text/css'}

# Every file under dist, subfolders too (trunk puts wasm-bindgen snippets
# in snippets/), by its path with / separators
files = sorted(os.path.relpath(os.path.join(d, f), DIST).replace(os.sep, '/')
               for d, _, names in os.walk(DIST) for f in names if os.path.splitext(f)[1] in TYPES)
if 'index.html' not in files:
    sys.exit(f'no index.html in {DIST}: build it with `trunk build --release` in tui/')
lines = ['// Made by esp32/make_web_files.py from tui/dist: do not edit.',
         '// atutop in the browser, gzipped.', '#pragma once', '#include <pgmspace.h>', '']
table = []
total = 0
for i, name in enumerate(files):
    data = gzip.compress(open(os.path.join(DIST, name), 'rb').read(), 9, mtime=0)
    total += len(data)
    lines.append(f'// {name}: {len(data)} bytes gzipped')
    lines.append(f'static const uint8_t WEB_FILE_{i}[] PROGMEM = {{')
    for j in range(0, len(data), 24):
        lines.append('  ' + ','.join(str(b) for b in data[j:j + 24]) + ',')
    lines.append('};')
    path = '/' if name == 'index.html' else '/' + name
    table.append(f'  {{"{path}", "{TYPES[os.path.splitext(name)[1]]}", WEB_FILE_{i}, sizeof WEB_FILE_{i}}},')
lines += ['', 'struct WebFile { const char *path; const char *type; const uint8_t *data; size_t size; };',
          'static const WebFile WEB_FILES[] = {'] + table + ['};',
          f'static const size_t WEB_FILE_COUNT = {len(files)};', '']
open(OUT, 'w').write('\n'.join(lines))
print(f'{OUT}: {len(files)} files, {total} bytes gzipped')
