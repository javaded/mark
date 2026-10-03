//! Export engine for Mark.
//!
//! Layer D of the architecture (plan.md §3): writes the final signed
//! document (Phase 9) and persists the signature/stamp asset library
//! ([`library`], Phase 6). Never overwrites the original input.

pub mod library;
