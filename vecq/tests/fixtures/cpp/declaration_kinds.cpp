// Every C++ declaration kind reachable by the extractor.
#include <string>

namespace coverage {

class Thing {
public:
    int value() const { return 1; }
};

}  // namespace coverage

int free_function() { return 0; }
