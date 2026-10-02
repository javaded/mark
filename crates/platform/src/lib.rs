//! Platform integration for Mark.
//!
//! Native file dialogs (`rfd`), application data directories, and any
//! genuinely platform-specific code. `#[cfg(target_os = ...)]` lives here
//! and nowhere else (plan.md §19.2). Arrives in Phase 3.
