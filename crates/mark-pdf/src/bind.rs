//! Runtime binding to the PDFium library (plan.md §6.3).
//!
//! Binding order mirrors `Pdfium::default()` semantics: the vendored
//! library from `script/fetch-pdfium.sh` first, then the system library.
//!
//! pdfium-render keeps the loaded library in a process-global cell: only
//! the *first* bind can load it, and any `Pdfium` value shares that global
//! (all calls serialize through the `thread_safe` mutex). So this module
//! binds **once per process** and hands every caller — worker thread,
//! test binary, example — the same `&'static Pdfium`.

use std::path::PathBuf;
use std::sync::OnceLock;

use pdfium_render::prelude::Pdfium;

/// First `vendor/pdfium/<platform>/lib` directory created by
/// `script/fetch-pdfium.sh`.
///
/// Resolved from the working directory (app and examples run from the
/// workspace root) *and* from this crate's manifest path (test binaries
/// run from `crates/mark-pdf`, where the relative lookup alone would miss
/// the fetched runtime and silently skip every integration test).
pub fn vendor_lib_dir() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("vendor/pdfium"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/pdfium"),
    ];
    let platform_lib = |dir: &PathBuf| {
        std::fs::read_dir(dir).ok()?.flatten().find_map(|platform| {
            let lib = platform.path().join("lib");
            lib.is_dir().then_some(lib)
        })
    };
    candidates.iter().find_map(platform_lib)
}

/// The process-wide PDFium instance: the first caller loads the vendored
/// runtime (falling back to the system library); later callers share it.
/// Returns `None` when neither is available.
static PDFIUM: OnceLock<Option<Pdfium>> = OnceLock::new();

/// Binds to the PDFium runtime (once per process), falling back to the
/// system library. Returns `None` when neither is available.
pub fn try_bind() -> Option<&'static Pdfium> {
    PDFIUM
        .get_or_init(|| {
            let bindings = vendor_lib_dir()
                .map(|dir| {
                    Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&dir))
                })
                .transpose()
                .ok()
                .flatten()
                .or_else(|| Pdfium::bind_to_system_library().ok());
            bindings.map(Pdfium::new)
        })
        .as_ref()
}
