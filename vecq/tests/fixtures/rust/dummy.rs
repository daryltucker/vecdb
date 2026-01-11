/// A complex Rust fixture for Big Boi testing
/// This file exercises the parser's ability to handle:
/// - Doc attributes
/// - Generics
/// - Lifetimes
/// - Macros
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct ComplexStruct<'a, T>
where T: Display {
    pub name: &'a str,
    pub data: T,
    map: HashMap<String, usize>,
}

pub enum State<T> {
    Idle,
    Processing(T),
    Error { code: i32, msg: String },
}

pub trait Processor {
    type Output;
    fn process(&self) -> Self::Output;
}

impl<'a, T> Processor for ComplexStruct<'a, T> 
where T: Display {
    type Output = String;
    
    fn process(&self) -> String {
        println!("Processing {}", self.name);
        format!("Processed: {}", self.data)
    }
}

mod internal {
    pub const VERSION: &str = "1.0.0";
    
    pub fn helper() {
        // Inner function
        let x = |a: i32| a * 2;
    }
}

#[macro_export]
macro_rules! my_macro {
    ($val:expr) => {
        println!("Value: {}", $val);
    };
}
