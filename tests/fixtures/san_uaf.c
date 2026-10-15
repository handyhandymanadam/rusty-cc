#include <stdlib.h>
int *dangling(void) { int *p = malloc(8); *p = 1; free(p); return p; }
int main(void) { int *q = dangling(); return *q; }
