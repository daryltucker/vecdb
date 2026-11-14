// Depth fixture: one binding or construct per addressable level, with
// distinct names so the depth test can ask "is X reachable?" per cell.
const X: i32 = 11;

fn f() -> i32 {
    let mut y = 33;
    let u;
    u = 2;
    y = 2;
    if y > 0 {
        let b = 44;
        block_call("block-str");
    }
    fn h() {}
    body_call("body-str");
    y + u
}

struct C {
    z: i32,
}

impl C {
    const K: i32 = 22;

    fn m(&self) -> i32 {
        let w = 55;
        method_call("method-str");
        w + self.z
    }
}
