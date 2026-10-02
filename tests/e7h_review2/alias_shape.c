// E7h review 2: two aliasing shapes, run as `./a.out 1` or `./a.out 2`. v2 is
// the alias plant's: a length stored as int, overwritten through a short lvalue,
// read back as int before the buffer the short view sized is written. GCC 16.2.1
// and GCC 13.3.0 (gcc:13.3, CI's) at -O3 forward the int store (n=64) plainly,
// under ASan and under ASan with UBSan; -fno-strict-aliasing reloads (n=4). v1,
// with the allocation between the stores and the load, is not forwarded.
#include <stdio.h>
#include <stdlib.h>
static char *volatile plant_sink;
static volatile unsigned long plant_spin;
static volatile int plant_one = 1;
__attribute__((noinline)) void alias_v1(void) {
    int *len = malloc(sizeof(int));
    plant_sink = (char *)len;
    short *alias = (short *)(void *)plant_sink;
    *len = 64;
    *alias = 4;
    char *buf = malloc(4);
    int n = *len;
    for (int i = 0; i < n; i++) buf[i] = (char)plant_one;
    plant_sink = buf;
    plant_spin += (unsigned long)n;
    fprintf(stderr, "v1 n=%d\n", n);
    free(buf);
    free(len);
}
__attribute__((noinline)) void alias_v2(void) {
    /* buffer sized through the short view, loop bound through the int view */
    int *len = malloc(sizeof(int));
    plant_sink = (char *)len;
    short *alias = (short *)(void *)plant_sink;
    *len = 64;
    *alias = 4;
    int n = *len;
    char *buf = malloc((size_t)(unsigned short)*alias);
    plant_sink = buf;
    char *volatile out = buf;
    for (int i = 0; i < n; i++) out[i] = (char)plant_one;
    plant_spin += (unsigned long)n;
    fprintf(stderr, "v2 n=%d\n", n);
    free(buf);
    free(len);
}
int main(int c, char **v) { if (c > 1 && v[1][0] == '2') alias_v2(); else alias_v1(); return 0; }
