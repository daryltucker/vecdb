// Depth fixture: one binding or construct per addressable level, with
// distinct names so the depth test can ask "is X reachable?" per cell.
int X = 11;
int U;

__global__ void k() {
    int q = 66;
}

int f() {
    int y = 33;
    y = 2;
    if (y) {
        int b = 44;
        block_call("block-str");
    }
    body_call("body-str");
    return y;
}

class C {
    int z = 22;
    int m() {
        int w = 55;
        method_call("method-str");
        return w;
    }
};
