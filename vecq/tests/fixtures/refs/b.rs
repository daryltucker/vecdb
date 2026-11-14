use crate::a::alpha;

pub fn beta() -> u32 {
    // alpha() in a comment is not a call
    let s = "alpha()";
    alpha() + crate::a::alpha() + alphabet()
}

impl Thing {
    fn m(&self) -> u32 {
        assert_eq!(alpha(), 1);
        self.alpha()
    }
}
