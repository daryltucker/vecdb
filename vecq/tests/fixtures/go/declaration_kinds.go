// Every Go declaration kind reachable by the extractor.
package coverage

import "fmt"

type Thing struct {
	A int
}

type Doer interface {
	Go()
}

func FreeFunction() {
	fmt.Println("hi")
}
