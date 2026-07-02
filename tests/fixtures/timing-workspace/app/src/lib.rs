//! App fixture crate: depends on `leaf` so a build compiles two crates.

/// Wrap the leaf crate's greeting so `leaf` is a real build dependency.
pub fn describe() -> String {
    format!("app on top of {}", leaf::greet())
}
