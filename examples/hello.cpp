#include <iostream>
#include <vector>
#include <numeric>

int main(int argc, char **argv) {
    const std::vector<int> values{1, 2, 3, 4, 5};
    std::cout << "Hello C++! GCC " << __VERSION__
              << "; __cplusplus=" << __cplusplus
              << "; sum=" << std::accumulate(values.begin(), values.end(), 0)
              << '\n';
    for (int i = 1; i < argc; ++i) {
        std::cout << "argv[" << i << "]=<" << argv[i] << ">\n";
    }
}
