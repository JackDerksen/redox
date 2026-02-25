//! Legacy compatibility module for higher-level editor logic.
//!
//! New code should prefer using the top-level `redox-core::motion` module.
//! This `logic` module exists as a thin re-export layer so older call sites
//! can keep importing `redox-core::logic::*` without churn.

pub use crate::motion::*;
