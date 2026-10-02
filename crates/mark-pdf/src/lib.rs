//! PDF document engine for Mark.
//!
//! Layer C of the architecture (plan.md §3): the only crate that talks to
//! PDFium, always through a single dedicated worker thread (plan.md §6.4).
//!
//! Phase 0: runtime binding only. Loading, rendering, and export arrive in
//! Phases 4 and 9.
