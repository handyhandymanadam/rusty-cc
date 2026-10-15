#include <stdio.h>
Foo make(void) { Foo f; return f; }
int main(void) {
    int a = missing + 1;
    int b = missing * 2;
    printf("%d\n", missing);
    return a + b;
}
