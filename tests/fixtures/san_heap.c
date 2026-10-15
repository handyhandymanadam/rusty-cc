#include <stdio.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    printf("before\n"); fflush(stdout);
    int *p = malloc(4 * sizeof(int));
    p[argc + 3] = 7;
    free(p);
    return 0;
}
