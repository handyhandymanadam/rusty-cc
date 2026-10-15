#include <stdio.h>
static int unused_fn(void) { return 1; }
int main(int argc, char **argv) {
    int unused;
    long n = 5;
    unsigned u = 3;
    if (n < u) printf("%d\n", n);
    return 0;
}
