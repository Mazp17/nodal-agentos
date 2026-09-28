//! Parsing of the final JSON block the prompt asks executors for.
//!
//! Moved to `nodal_domain::execution::report`; re-exported here so current uses don't break.
//! Only `work::pump::tests` still reaches it through this path.

#[allow(unused_imports)]
pub use nodal_domain::execution::report::*;

#[cfg(test)]
mod tests;
