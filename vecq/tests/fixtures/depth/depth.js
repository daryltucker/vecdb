// Depth fixture: one binding or construct per addressable level, with
// distinct names so the depth test can ask "is X reachable?" per cell.
var X = 11;
let U;
top_call("top-str");

function f() {
  let y = 33;
  y = 2;
  if (y) {
    let b = 44;
    block_call("block-str");
  }
  function h() {}
  body_call("body-str");
  return y;
}

class C {
  z = 22;
  m() {
    let w = 55;
    method_call("method-str");
    return w;
  }
}
