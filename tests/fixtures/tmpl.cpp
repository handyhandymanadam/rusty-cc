#include <vector>
#include <map>
#include <string>
template <typename T> struct Box { T v; int size() const { return v.size(); } };
int main() {
    Box<int> b{3};
    std::map<std::string, std::vector<int>> m;
    m["a"].push_back("not an int");
    return b.size();
}
