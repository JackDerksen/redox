//! Legacy compatibility module for higher-level editor logic.
//!
//! New code should prefer using the top-level `editor_core::motion` module.
//! This `logic` module exists as a thin re-export layer so older call sites
//! can keep importing `editor_core::logic::*` without churn.

pub use crate::motion::*;
