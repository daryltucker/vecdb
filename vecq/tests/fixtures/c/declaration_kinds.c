/* Every C declaration kind reachable by the extractor. */
#include <stdio.h>

#define MAX 10
#define SQUARE(x) ((x) * (x))

struct Thing {
    int a;
};

int free_function(void) {
    return MAX;
}
