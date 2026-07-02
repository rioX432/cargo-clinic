//! Leaf fixture crate: a trivial, dependency-free unit for timing tests.

/// Return a constant greeting. Exists only to give rustc something to compile.
pub fn greet() -> &'static str {
    "leaf"
}
