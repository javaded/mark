//! Verifies a PDFium runtime can be located and bound.
//!
//! Run `script/fetch-pdfium.sh` first. Optionally pass a directory containing
//! the PDFium library as the first argument.

use pdfium_render::prelude::*;
use std::path::PathBuf;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_vendor_dir);

    let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&dir))
        .or_else(|_| Pdfium::bind_to_system_library())
        .unwrap_or_else(|e| {
            eprintln!(
                "Could not bind PDFium (looked in {}, then system paths).",
                dir.display()
            );
            eprintln!("Run script/fetch-pdfium.sh first. Error: {e:?}");
            std::process::exit(1);
        });

    let pdfium = Pdfium::new(bindings);
    // Exercise the binding with a real FPDF call, not just dlopen.
    let document = pdfium
        .create_new_pdf()
        .expect("PDFium bound but FPDF_CreateNewDocument failed");
    println!(
        "PDFium bound successfully (created empty document, {} pages).",
        document.pages().len()
    );
    println!("Library directory: {}", dir.display());
}

/// First `vendor/pdfium/<platform>/lib` directory created by fetch-pdfium.sh.
fn default_vendor_dir() -> PathBuf {
    if let Ok(platforms) = std::fs::read_dir("vendor/pdfium") {
        for platform in platforms.flatten() {
            let lib = platform.path().join("lib");
            if lib.is_dir() {
                return lib;
            }
        }
    }
    PathBuf::from("vendor/pdfium")
}
