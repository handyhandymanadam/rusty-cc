#include <stdio.h>
#include <stdlib.h>
static int small(int x) { return x + 1; }
__attribute__((noinline)) int never(int x) { return x * 3; }
int big(int x) { int s = 0; for (int i = 0; i < x; i++) { s += i * x; if (s > 1000) s -= 7; s ^= x; s += rand(); } return s; }
int rec(int n) { return n <= 1 ? 1 : n * rec(n - 1); }
int vec(int *a, int *b, int n) { for (int i = 0; i < n; i++) a[i] += b[i]; return a[0]; }
int nov(int *a, int n) { int s = 0; for (int i = 0; i < n; i++) { s += a[i]; if (s == 77) printf("x"); } return s; }
int main(int argc, char **argv) { int a[100] = {0}, b[100] = {0}; return small(argc) + never(argc) + big(argc) + big(argv[0][0]) + rec(argc) + vec(a, b, argc) + nov(a, argc); }
