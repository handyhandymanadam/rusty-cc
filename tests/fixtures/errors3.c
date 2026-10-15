#include <stdio.h>
int f(void) {
    int x = undefined_name + 1;
    return x
}
void g(void) {
    char *p = 5;
    printf("%d\n", nosuch(p));
}
