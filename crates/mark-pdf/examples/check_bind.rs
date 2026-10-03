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
        .or_else(mark_pdf::bind::vendor_lib_dir)
        .unwrap_or_else(|| PathBuf::from("vendor/pdfium"));

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
