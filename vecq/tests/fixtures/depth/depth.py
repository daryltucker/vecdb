# Depth fixture: one binding or construct per addressable level, with
# distinct names so the depth test can ask "is X reachable?" per cell.
X = 11
U: int
top_call("top-str")


def f():
    y = 33
    y = 2
    if y:
        b = 44
        block_call("block-str")

    def h():
        pass

    body_call("body-str")
    return y


class C:
    z = 22

    def m(self):
        w = 55
        method_call("method-str")
        return w
