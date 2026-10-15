template <int N> struct Fact { static_assert(N != 3, "reached three"); static const int value = N * Fact<N - 1>::value; };
template <> struct Fact<0> { static const int value = 1; };
void over(int, int);
void over(int, long);
void over(long, int);
void over(char *, int);
void over(double, double, double);
void over(const char *);
int main() {
    over("a", "b");
    return Fact<7>::value;
}
