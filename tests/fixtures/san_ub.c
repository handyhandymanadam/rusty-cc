#include <stdio.h>
int table[4];
int get(int i) { return table[i]; }
int main(int argc, char **argv) {
    int big = 2147483647;
    int sum = big + argc;
    printf("sum=%d get=%d\n", sum, get(argc + 5));
    return 3;
}
