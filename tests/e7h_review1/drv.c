// E7h review 1: parse a file cold (N times with a count), then retype it one
// byte at a time with an incremental reparse after each (the typing path).
// Build with -DLANG=tree_sitter_<name>. Usage: drv FILE [cold|retype|both] [N]
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <tree_sitter/api.h>

#ifndef LANG
#define LANG tree_sitter_haskell
#endif
const TSLanguage *LANG(void);

static char *slurp(const char *path, size_t *len) {
  FILE *f = fopen(path, "rb");
  if (!f) { perror(path); exit(2); }
  fseek(f, 0, SEEK_END);
  long n = ftell(f);
  fseek(f, 0, SEEK_SET);
  char *buf = malloc((size_t)n + 1);
  if (fread(buf, 1, (size_t)n, f) != (size_t)n) { perror("read"); exit(2); }
  fclose(f);
  buf[n] = 0;
  *len = (size_t)n;
  return buf;
}

int main(int argc, char **argv) {
  if (argc < 2) { fprintf(stderr, "usage: drv FILE [cold|retype|both]\n"); return 2; }
  const char *mode = argc > 2 ? argv[2] : "both";
  size_t len;
  char *src = slurp(argv[1], &len);
  TSParser *p = ts_parser_new();
  ts_parser_set_language(p, LANG());
  unsigned parses = 0, null_trees = 0;
  int cold_err = -1;
  int reps = argc > 3 ? atoi(argv[3]) : 1;
  if (strcmp(mode, "retype") != 0) for (int r = 0; r < reps; r++) {
    TSTree *t = ts_parser_parse_string(p, NULL, src, (uint32_t)len);
    parses++;
    if (!t) null_trees++;
    else { cold_err = ts_node_has_error(ts_tree_root_node(t)); ts_tree_delete(t); }
  }
  if (strcmp(mode, "cold") != 0) {
    TSTree *t = ts_parser_parse_string(p, NULL, "", 0);
    parses++;
    uint32_t row = 0, col = 0;
    for (size_t i = 0; i < len; i++) {
      TSPoint at = {row, col};
      TSPoint after = src[i] == '\n' ? (TSPoint){row + 1, 0} : (TSPoint){row, col + 1};
      TSInputEdit e = {
        .start_byte = (uint32_t)i, .old_end_byte = (uint32_t)i, .new_end_byte = (uint32_t)i + 1,
        .start_point = at, .old_end_point = at, .new_end_point = after,
      };
      if (t) ts_tree_edit(t, &e);
      TSTree *n = ts_parser_parse_string(p, t, src, (uint32_t)i + 1);
      parses++;
      if (!n) null_trees++;
      if (t) ts_tree_delete(t);
      t = n;
      row = after.row; col = after.column;
    }
    if (t) ts_tree_delete(t);
  }
  printf("ok: %zu bytes, %u parses, %u without a tree, cold has_error=%d\n", len, parses, null_trees, cold_err);
  ts_parser_delete(p);
  free(src);
  return 0;
}
