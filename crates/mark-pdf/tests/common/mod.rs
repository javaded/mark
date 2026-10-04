//! Shared helpers for mark-pdf integration tests.
//!
//! Tests that need the real PDFium runtime skip gracefully when it is not
//! present (plan.md §20.5): CI fetches it on Linux via
//! `script/fetch-pdfium.sh`; macos/windows domain runs skip.

// Helpers are shared across test binaries; not every binary uses every one.
#![allow(dead_code)]

use std::path::PathBuf;

use image::{Rgba, RgbaImage};
use pdfium_render::prelude::Pdfium;

pub fn pdfium_or_skip() -> Option<&'static Pdfium> {
    let pdfium = mark_pdf::bind::try_bind();
    if pdfium.is_none() {
        eprintln!("skipping: no PDFium runtime (run script/fetch-pdfium.sh)");
    }
    pdfium
}

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../resources/test-documents")
        .join(name)
}

/// Samples the pixel at fractional bitmap coordinates `(fx, fy)` of the
/// image, pulled toward the center so edge antialiasing never bites.
pub fn sample(image: &RgbaImage, fx: f64, fy: f64) -> Rgba<u8> {
    let x = ((fx * image.width() as f64) as u32).clamp(1, image.width() - 2);
    let y = ((fy * image.height() as f64) as u32).clamp(1, image.height() - 2);
    *image.get_pixel(x, y)
}

pub fn assert_color(actual: Rgba<u8>, expected: [u8; 3], context: &str) {
    let [r, g, b] = expected;
    assert!(
        actual.0[0].abs_diff(r) <= 8
            && actual.0[1].abs_diff(g) <= 8
            && actual.0[2].abs_diff(b) <= 8,
        "{context}: expected ({r}, {g}, {b}), got {:?}",
        actual.0
    );
}
