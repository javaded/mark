//! Runtime binding to the PDFium library (plan.md §6.3).
//!
//! Binding order mirrors `Pdfium::default()` semantics: the vendored
//! library from `script/fetch-pdfium.sh` first, then the system library.

use std::path::PathBuf;

use pdfium_render::prelude::Pdfium;

/// First `vendor/pdfium/<platform>/lib` directory created by
/// `script/fetch-pdfium.sh`, relative to the working directory.
pub fn vendor_lib_dir() -> Option<PathBuf> {
    std::fs::read_dir("vendor/pdfium")
        .ok()?
        .flatten()
        .find_map(|platform| {
            let lib = platform.path().join("lib");
            lib.is_dir().then_some(lib)
        })
}

/// Binds to the vendored PDFium runtime, falling back to the system
/// library. Returns `None` when neither is available.
pub fn try_bind() -> Option<Pdfium> {
    let bindings = vendor_lib_dir()
        .map(|dir| Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&dir)))
        .transpose()
        .ok()
        .flatten()
        .or_else(|| Pdfium::bind_to_system_library().ok())?;
    Some(Pdfium::new(bindings))
}
