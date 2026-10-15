#include <stdlib.h>
int *keep;
void leak(void) { keep = malloc(100); keep = NULL; }
int main(void) { leak(); return 0; }
