//! Every Rust declaration kind, so `tier1_declaration_coverage` can prove each
//! one is reachable. Do not prune: a kind removed here silently un-tests itself.
use std::collections::HashMap;

/// A documented enum.
pub enum Kind { A, B }

/// A documented trait.
pub trait Doer {
    fn go(&self);
}

pub type Alias = HashMap<String, u32>;
pub const MAX: u32 = 10;
pub static GLOBAL: u32 = 20;

pub union Overlap {
    integer: u32,
    float: f32,
}

macro_rules! noop {
    () => {};
}

/// A documented struct.
pub struct Thing;

impl Thing {
    fn method(&self) {}
}

pub fn free_function() {}

pub mod inner {}
