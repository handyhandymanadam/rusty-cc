struct V { int a; };
int f(struct V v) {
    if (v.a = 1) return 1;
    return 0;
}
