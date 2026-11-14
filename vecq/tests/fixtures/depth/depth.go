// Depth fixture: one binding or construct per addressable level, with
// distinct names so the depth test can ask "is X reachable?" per cell.
package depth

var X = 11
var U int

func f() int {
	y := 33
	y = 2
	if y > 0 {
		b := 44
		block_call("block-str")
		_ = b
	}
	body_call("body-str")
	return y
}

type C struct {
	z int
}

func (c C) m() int {
	w := 55
	method_call("method-str")
	return w
}
