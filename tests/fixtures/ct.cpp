#include <map>
#include <string>
#include <vector>
#include <algorithm>
#include <iostream>
#include <regex>
template <int N> struct Fib { static const long v = Fib<N-1>::v + Fib<N-2>::v; };
template <> struct Fib<1> { static const long v = 1; };
template <> struct Fib<0> { static const long v = 0; };
int slow(int x) { std::map<std::string, std::vector<int>> m; for (int i = 0; i < x; i++) m[std::to_string(i)].push_back(i); std::regex r("a+b"); return (int)m.size() + (int)Fib<20>::v + (int)std::regex_match("aab", r); }
int main() { return slow(3); }
