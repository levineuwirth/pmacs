## Keystroke to visible update (ms), pooled samples

| file | build | arm | n | p50 | p95 | p99 | per-round p50 | loads |
|---|---|---|---|---|---|---|---|---|
| rust | built | none | 240 | 203 | 274 | 360 | 217, 238, 223, 154, 155, 200 | 9.0, 10.8, 9.6, 6.2, 5.5, 7.3 |
| rust | built | process | 240 | 187 | 258 | 384 | 212, 223, 216, 190, 150, 152 | 9.3, 11.9, 9.1, 7.5, 5.4, 5.6 |
| rust | proto | process | 240 | 186 | 262 | 298 | 206, 225, 163, 226, 166, 154 | 10.7, 10.4, 10.5, 9.1, 6.2, 6.7 |
| rust | proto | none | 240 | 171 | 304 | 519 | 194, 178, 218, 154, 157, 137 | 9.8, 14.6, 9.9, 6.5, 5.4, 6.1 |
| markdown | built | none | 240 | 539 | 815 | 983 | 744, 497, 561, 527, 502, 495 | 12.1, 7.6, 10.2, 8.4, 5.7, 5.7 |
| markdown | built | process | 240 | 518 | 1292 | 1363 | 1230, 512, 510, 714, 478, 482 | 22.0, 8.6, 8.0, 9.0, 5.7, 5.7 |
| markdown | proto | process | 240 | 526 | 1017 | 1228 | 836, 478, 608, 515, 537, 496 | 13.2, 6.2, 13.9, 7.1, 6.4, 7.7 |
| markdown | proto | none | 240 | 523 | 1075 | 1339 | 988, 518, 534, 489, 508, 508 | 19.4, 6.1, 8.9, 6.9, 5.9, 6.2 |

## Against native (built none), by the comparison's band

| file | arm | quantile | value | native | diff | band | verdict |
|---|---|---|---|---|---|---|---|
| rust | built process | p50 | 187 | 203 | -16 | 84 | inside |
| rust | built process | p95 | 258 | 274 | -16 | 139 | inside |
| rust | built process | p99 | 384 | 360 | +23 | 322 | inside |
| rust | proto process | p50 | 186 | 203 | -16 | 84 | inside |
| rust | proto process | p95 | 262 | 274 | -12 | 139 | inside |
| rust | proto process | p99 | 298 | 360 | -62 | 322 | inside |
| rust | proto none | p50 | 171 | 203 | -32 | 84 | inside |
| rust | proto none | p95 | 304 | 274 | +30 | 139 | inside |
| rust | proto none | p99 | 519 | 360 | +159 | 322 | inside |
| markdown | built process | p50 | 518 | 539 | -22 | 248 | inside |
| markdown | built process | p95 | 1292 | 815 | +477 | 496 | inside |
| markdown | built process | p99 | 1363 | 983 | +380 | 957 | inside |
| markdown | proto process | p50 | 526 | 539 | -12 | 248 | inside |
| markdown | proto process | p95 | 1017 | 815 | +202 | 496 | inside |
| markdown | proto process | p99 | 1228 | 983 | +245 | 957 | inside |
| markdown | proto none | p50 | 523 | 539 | -16 | 248 | inside |
| markdown | proto none | p95 | 1075 | 815 | +259 | 496 | inside |
| markdown | proto none | p99 | 1339 | 983 | +356 | 957 | inside |

## Paired by round: built process minus proto process, and built process minus built none (p50, ms)

- rust: built-proto +6, -2, +53, -36, -16, -2; built-native -4, -15, -7, +36, -5, -48
- markdown: built-proto +394, +34, -98, +200, -58, -14; built-native +486, +15, -52, +188, -24, -14

## Responsiveness during a pathological parse

| victim | build | arm | n | typed p50 | p99 | caret p50 | p99 | deaths (kinds) | peak MB | last MB | alive |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 296-16k | built | process | 90 | 170 | 355 | 18 | 32 | 60 (ended,memory) | 1088 | 1077 | True |
| 296-16k | proto | process | 90 | 168 | 470 | 19 | 52 | 31 (memory) | 1088 | 1079 | True |
| 301 | built | none | 90 | 221 | 336 | 26 | 43 | 0 () | 135 | 129 | True |
| 301 | built | process | 90 | 174 | 274 | 23 | 39 | 16 (ended,time) | 83 | 83 | True |
| 301 | proto | process | 90 | 178 | 516 | 22 | 42 | 16 (time) | 80 | 70 | True |

## Steady state: eleven buffers, PSS after typing (MB)

| build | arm | runs | total PSS each | median | daemon PSS | workers |
|---|---|---|---|---|---|---|
| built | none | 3 | 194.1, 189.0, 198.5 | 194.1 | 194.1, 189.0, 198.5 | 0, 0, 0 |
| built | process | 3 | 158.9, 159.5, 158.3 | 158.9 | 47.8, 48.4, 47.1 | 11, 11, 11 |
| proto | process | 3 | 155.7, 157.2, 154.9 | 155.7 | 48.1, 49.6, 47.2 | 11, 11, 11 |

## Acceptance, once, the built tip in process mode

- 296-16k: deaths 1 ['memory'] at [1589] ms; peak 1054 MB, last 64 MB; typed visible n=5 min=209 p50=225 p90=237 p95=237 p99=237 max=237; alive True; leftover workers []
- 301: deaths 1 ['time'] at [5101] ms; peak 78 MB, last 74 MB; typed visible n=5 min=199 p50=229 p90=279 p95=279 p99=279 max=279; alive True; leftover workers []
- nest-3x8k: deaths 0 [] at [] ms; peak 993 MB, last 918 MB; typed visible n=5 min=133 p50=150 p90=154 p95=154 p99=154 max=154; alive True; leftover workers []
- 296-32k: deaths 1 ['memory'] at [1063] ms; peak 1051 MB, last 73 MB; typed visible n=5 min=124 p50=130 p90=143 p95=143 p99=143 max=143; alive True; leftover workers []
- 296-16k,296-16k,296-16k,296-16k,296-16k,296-16k: deaths 6 ['memory', 'total'] at [498, 508, 508, 510, 510, 1224] ms; peak 1737 MB, last 65 MB; typed visible n=5 min=126 p50=133 p90=134 p95=134 p99=134 max=134; alive True; leftover workers []

## In-unit parse and the boundary, per keystroke (ms), the open's cold parse excluded

| file | build | arm | parses | in-unit p50 | p95 | boundary p50 (call - unit) |
|---|---|---|---|---|---|---|
| rust | built | none | 960 | 82.0 | 111.7 | - |
| rust | built | process | 960 | 75.9 | 109.9 | 0.31 |
| rust | proto | process | 960 | 76.7 | 106.9 | 2.45 |
| rust | proto | none | 960 | 66.1 | 117.6 | - |
| markdown | built | none | 960 | 241.9 | 379.5 | - |
| markdown | built | process | 960 | 244.8 | 600.2 | 6.52 |
| markdown | proto | process | 960 | 237.4 | 462.3 | 14.12 |
| markdown | proto | none | 960 | 237.5 | 522.6 | - |
