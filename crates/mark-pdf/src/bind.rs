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

/// The `vendor/pdfium/<platform>` directory name the fetch script uses
/// for the *current* platform, so a machine with several runtimes fetched
/// (cross-platform dev checkout) binds the right one.
fn vendor_platform_dir() -> String {
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unknown"
    };
    if cfg!(target_os = "linux") {
        if cfg!(target_env = "musl") {
            format!("linux-musl-{arch}")
        } else {
            format!("linux-{arch}")
        }
    } else if cfg!(target_os = "macos") {
        format!("mac-{arch}")
    } else if cfg!(target_os = "windows") {
        format!("win-{arch}")
    } else {
        format!("unknown-{arch}")
    }
}

/// Subdirectories a fetched runtime's library may live in: bblanchon's
/// archives use `lib/` on Linux/macOS but keep `pdfium.dll` in `bin/` on
/// Windows (`lib/` there holds only the import library `.lib`).
const VENDOR_LIB_SUBDIRS: [&str; 2] = ["lib", "bin"];

/// The library file name pdfium-render loads on this platform.
fn platform_library_file() -> &'static str {
    if cfg!(target_os = "windows") {
        "pdfium.dll"
    } else if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    }
}

/// First `vendor/pdfium/<platform>` library directory created by
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
    let library = platform_library_file();
    let dir_with_library = |dir: &PathBuf| -> Option<PathBuf> {
        // The current platform's runtime first; any fetched runtime as a
        // fallback (a wrong-arch library simply fails to load, not
        // silently skip — better to try the exact match before anything
        // else). A directory only counts when the library *file* is in it.
        let names = std::iter::once(vendor_platform_dir()).chain(
            std::fs::read_dir(dir)
                .ok()?
                .flatten()
                .filter_map(|platform| platform.file_name().into_string().ok()),
        );
        names
            .filter_map(|name| {
                VENDOR_LIB_SUBDIRS
                    .iter()
                    .map(|sub| dir.join(&name).join(sub))
                    .find(|lib| lib.join(library).is_file())
            })
            .next()
    };
    candidates.iter().find_map(dir_with_library)
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
