// E7h review 2: parse a file once natively through ts_parser_parse_with_options
// with a progress callback, and report the parse time, the callbacks, the
// longest gap between two, the time from the last one to the return, the byte
// offset the last one reported, and peak RSS. With a deadline in seconds as a
// second argument, the callback cancels past it, as pmacs's run_parse does.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <tree_sitter/api.h>
const TSLanguage *LANG(void);
static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }
static double t0, last, gap, deadline; static unsigned long calls; static uint32_t last_off, gap_from, gap_to;
static bool cb(TSParseState *s) {
  double t = now();
  if (t - last > gap) { gap = t - last; gap_from = last_off; gap_to = s->current_byte_offset; }
  last = t; calls++; last_off = s->current_byte_offset;
  return deadline > 0 && t - t0 > deadline;
}
typedef struct { const char *s; uint32_t n; } Src;
static const char *rd(void *p, uint32_t i, TSPoint pt, uint32_t *got) {
  (void)pt; Src *s = p; if (i >= s->n) { *got = 0; return ""; } *got = s->n - i; return s->s + i;
}
static long hwm(void) { FILE *f = fopen("/proc/self/status", "r"); char l[256]; long kb = 0;
  while (fgets(l, sizeof l, f)) if (!strncmp(l, "VmHWM:", 6)) kb = atol(l + 6); fclose(f); return kb; }
int main(int argc, char **argv) {
  FILE *f = fopen(argv[1], "rb"); fseek(f, 0, SEEK_END); long n = ftell(f); fseek(f, 0, SEEK_SET);
  char *buf = malloc(n + 1); if (fread(buf, 1, n, f) != (size_t)n) return 2; fclose(f);
  deadline = argc > 2 ? atof(argv[2]) : 0;
  TSParser *p = ts_parser_new(); ts_parser_set_language(p, LANG());
  Src src = { buf, (uint32_t)n };
  TSInput in = { .payload = &src, .read = rd, .encoding = TSInputEncodingUTF8 };
  TSParseOptions o = { .payload = NULL, .progress_callback = cb };
  t0 = now(); last = t0;
  TSTree *t = ts_parser_parse_with_options(p, NULL, in, o);
  double t1 = now();
  printf("%ld bytes: %s in %.3f s; %lu callbacks, longest gap %.3f s (offset %u to %u), %.3f s after the last (offset %u of %ld); peak %ld MB\n",
         n, t ? "tree" : "cancelled", t1 - t0, calls, gap, gap_from, gap_to, t1 - last, last_off, n, hwm() / 1024);
  if (t) ts_tree_delete(t);
  return 0;
}
