/* Depth fixture: one binding or construct per addressable level, with
   distinct names so the depth test can ask "is X reachable?" per cell. */
int X = 11;
int U;

int f(void) {
    int y = 33;
    y = 2;
    if (y) {
        int b = 44;
        block_call("block-str");
    }
    body_call("body-str");
    return y;
}

struct C {
    int z;
};
